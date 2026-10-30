#![no_std]
#![no_main]
#[path = "../../shared/elf.rs"]
mod elf;
#[path = "../../shared/init_protocol.rs"]
mod protocol;
#[path = "../../shared/service_protocol.rs"]
mod service;
#[path = "../../shared/startup.rs"]
mod startup;
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("../common/riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("../common/x86_64.S"));
unsafe extern "C" {
    fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize;
}
fn call(n: usize, a: [usize; 6]) -> isize {
    unsafe { syscall6(n, a[0], a[1], a[2], a[3], a[4], a[5]) }
}
fn exit(status: usize) -> ! {
    call(1, [status, 0, 0, 0, 0, 0]);
    loop {
        core::hint::spin_loop()
    }
}
fn checked(n: usize, a: [usize; 6]) -> usize {
    let r = call(n, a);
    if r < 0 {
        exit(10 + n)
    }
    r as usize
}
fn send(handle: usize, bytes: &[u8]) {
    checked(8, [handle, bytes.as_ptr() as usize, bytes.len(), 0, 0, 0]);
}
fn recv(handle: usize, b: &mut [u8]) {
    let mut counts = [0u32; 2];
    checked(
        10,
        [
            handle,
            b.as_mut_ptr() as usize,
            b.len(),
            0,
            0,
            counts.as_mut_ptr() as usize,
        ],
    );
    if counts != [b.len() as u32, 0] {
        exit(40)
    }
}
const IMAGE_BASE: usize = 0x3000_0000;
fn launch(job: usize, image_handle: usize, image_len: usize) -> usize {
    let size = image_len
        .checked_add(4095)
        .map(|n| n & !4095)
        .unwrap_or_else(|| exit(41));
    checked(14, [image_handle, 0, IMAGE_BASE, size, 1, 0]);
    let image_bytes = unsafe { core::slice::from_raw_parts(IMAGE_BASE as *const u8, image_len) };
    let (machine, limit) = if cfg!(target_arch = "riscv64") {
        (243, 1usize << 38)
    } else {
        (62, 1usize << 47)
    };
    let image = elf::ElfImage::parse(image_bytes, machine, limit).unwrap_or_else(|_| exit(42));
    let controller = checked(17, [job, 0, 0, 0, 0, 0]);
    for segment in image.segments() {
        if !segment.readable
            || segment.memory_size
                > segment.page_size - (segment.virtual_address - segment.page_start)
        {
            exit(42);
        }
        let options = if segment.executable { 3 } else { 1 };
        let vmo = checked(11, [segment.page_size, options, 0, 0, 0, 0]);
        let mut offset = 0;
        while offset < segment.data.len() {
            let length = (segment.data.len() - offset).min(65536);
            checked(
                13,
                [
                    vmo,
                    segment.virtual_address - segment.page_start + offset,
                    segment.data[offset..].as_ptr() as usize,
                    length,
                    0,
                    0,
                ],
            );
            offset += length;
        }
        if segment.executable {
            checked(16, [vmo, 0, 0, 0, 0, 0]);
        }
        let permissions = if segment.executable {
            5
        } else if segment.writable {
            3
        } else {
            1
        };
        let descriptor = [
            segment.page_start as u64,
            0,
            segment.page_size as u64,
            permissions,
            0,
            0,
        ];
        checked(18, [controller, vmo, descriptor.as_ptr() as usize, 0, 0, 0]);
        checked(2, [vmo, 0, 0, 0, 0, 0]);
    }
    let mut channel = [0u32; 2];
    checked(7, [channel.as_mut_ptr() as usize, 0, 0, 0, 0, 0]);
    let bootstrap = checked(19, [controller, channel[1] as usize, 0, 0, 0, 0]);
    checked(20, [controller, image.entry(), bootstrap, 0, 0, 0]);
    checked(21, [controller, 0, 0, 0, 0, 0]);
    checked(2, [controller, 0, 0, 0, 0, 0]);
    checked(15, [IMAGE_BASE, size, 0, 0, 0, 0]);
    checked(2, [image_handle, 0, 0, 0, 0, 0]);
    checked(2, [job, 0, 0, 0, 0, 0]);
    channel[0] as usize
}
#[no_mangle]
pub unsafe extern "C" fn user_main(record: *const u8) -> ! {
    if record.is_null() || record.addr() & 15 != 0 {
        exit(1)
    }
    let bootstrap =
        match startup::Startup::decode(core::slice::from_raw_parts(record, startup::SIZE)) {
            Ok(startup::Startup { bootstrap: Some(h) }) => h as usize,
            _ => exit(1),
        };
    let mut start = [0; protocol::SIZE];
    let mut handles = [0u32; 2];
    let mut counts = [0u32; 2];
    checked(
        10,
        [
            bootstrap,
            start.as_mut_ptr() as usize,
            start.len(),
            handles.as_mut_ptr() as usize,
            2,
            counts.as_mut_ptr() as usize,
        ],
    );
    let len = match protocol::decode_record(&start) {
        Ok((protocol::START, n))
            if n > 0 && n <= elf::MAX_IMAGE_SIZE as u64 && counts == [protocol::SIZE as u32, 2] =>
        {
            n as usize
        }
        _ => exit(2),
    };
    let endpoint = launch(handles[0] as usize, handles[1] as usize, len);
    let mut message = [0; service::SIZE];
    recv(endpoint, &mut message);
    if service::decode(&message) != Some((service::READY, 0)) {
        exit(3)
    }
    if cfg!(scenario = "init-replay-challenge") {
        send(endpoint, &service::encode(service::PING, 74));
        recv(endpoint, &mut message);
        if service::decode(&message) != Some((service::PONG, 74)) {
            exit(6)
        }
    }
    send(bootstrap, &protocol::encode(protocol::READY).unwrap());
    loop {
        recv(bootstrap, &mut message);
        match service::decode(&message) {
            Some((service::PING, value)) => {
                if cfg!(scenario = "init-replay-challenge") && value == 74 {
                    send(bootstrap, &service::encode(service::PONG, value));
                    continue;
                }
                send(endpoint, &message);
                recv(endpoint, &mut message);
                if service::decode(&message) != Some((service::PONG, value)) {
                    exit(4)
                }
                send(bootstrap, &message)
            }
            Some((service::SHUTDOWN, 0)) => {
                send(endpoint, &message);
                checked(0, [0; 6]);
                checked(2, [endpoint, 0, 0, 0, 0, 0]);
                checked(2, [bootstrap, 0, 0, 0, 0, 0]);
                exit(1)
            }
            _ => exit(5),
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(99)
}
