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
#[path = "../common/fs_client.rs"]
mod fs_client;
#[path = "../../shared/fs_protocol.rs"]
mod fs_protocol;
#[path = "../../shared/initramfs.rs"]
mod initramfs;
#[path = "../common/runtime.rs"]
mod rt;
#[path = "../../shared/services_config.rs"]
mod services_config;
use rt::*;
const IMAGE_BASE: usize = 0x3000_0000;
fn prepare(job: usize, image_bytes: &[u8]) -> (usize, usize) {
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
    (controller, image.entry())
}
fn start_process(controller: usize, entry: usize) -> usize {
    let channel = pair();
    let bootstrap = checked(19, [controller, channel[1] as usize, 0, 0, 0, 0]);
    checked(20, [controller, entry, bootstrap, 0, 0, 0]);
    checked(21, [controller, 0, 0, 0, 0, 0]);
    close(controller);
    channel[0] as usize
}
fn source(client: &mut fs_client::Client, path: &str) -> (usize, usize) {
    let (id, len) = client.open(path, fs_protocol::FILE);
    if len == 0 || len > elf::MAX_IMAGE_SIZE {
        exit(42)
    }
    let size = len.div_ceil(4096) * 4096;
    let vmo = checked(11, [size, 3, 0, 0, 0, 0]);
    let mut offset = 0;
    while offset < len {
        let count = client.read(id, offset, (len - offset).min(fs_protocol::CHUNK));
        if count == 0 {
            exit(42)
        }
        checked(
            13,
            [
                vmo,
                offset,
                client.buffer[fs_protocol::HEADER..].as_ptr() as usize,
                count,
                0,
                0,
            ],
        );
        offset += count;
        if client.flags == fs_protocol::EOF && offset != len {
            exit(42)
        }
    }
    client.close(id);
    checked(16, [vmo, 0, 0, 0, 0, 0]);
    (vmo, len)
}
fn prepare_source(job: usize, client: &mut fs_client::Client, path: &str) -> (usize, usize) {
    let (vmo, len) = source(client, path);
    let size = len.div_ceil(4096) * 4096;
    checked(14, [vmo, 0, IMAGE_BASE, size, 1, 0]);
    let result = prepare(job, unsafe {
        core::slice::from_raw_parts(IMAGE_BASE as *const u8, len)
    });
    checked(15, [IMAGE_BASE, size, 0, 0, 0, 0]);
    close(vmo);
    result
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
            if n > 0 && n <= initramfs::MAX_SIZE as u64 && counts == [protocol::SIZE as u32, 2] =>
        {
            n as usize
        }
        _ => exit(2),
    };
    let job = handles[0] as usize;
    let archive_handle = handles[1] as usize;
    let archive_size = len.div_ceil(4096) * 4096;
    checked(14, [archive_handle, 0, IMAGE_BASE, archive_size, 1, 0]);
    let archive =
        initramfs::Archive::parse(core::slice::from_raw_parts(IMAGE_BASE as *const u8, len))
            .unwrap_or_else(|_| exit(42));
    let fs_image = archive
        .find("sbin/initfs")
        .filter(|e| e.kind == initramfs::Kind::File)
        .unwrap_or_else(|| exit(42));
    let (fs_controller, fs_entry) = prepare(job, fs_image.data);
    let mut clients = [0u32; fs_protocol::MAX_SESSIONS];
    let mut server_slots = [0u32; fs_protocol::MAX_SESSIONS];
    for i in 0..clients.len() {
        let p = pair();
        clients[i] = p[0];
        server_slots[i] = checked(19, [fs_controller, p[1] as usize, 0, 0, 0, 0]) as u32;
    }
    let fs_control = start_process(fs_controller, fs_entry);
    checked(15, [IMAGE_BASE, archive_size, 0, 0, 0, 0]);
    let start = fs_protocol::fs_start(len as u64, &server_slots).unwrap();
    let grant = [archive_handle as u32];
    checked(
        8,
        [
            fs_control,
            start.as_ptr() as usize,
            start.len(),
            grant.as_ptr() as usize,
            1,
            0,
        ],
    );
    let mut message = [0; service::SIZE];
    recv(fs_control, &mut message);
    if service::decode(&message) != Some((service::READY, 0)) {
        exit(3)
    }
    let mut client = fs_client::Client::new(clients[0] as usize);
    let (config_id, config_len) = client.open("etc/services.conf", fs_protocol::FILE);
    if config_len == 0 || config_len > 4096 {
        exit(43)
    }
    let mut config = [0u8; 4096];
    let n = client.read(config_id, 0, config_len);
    if n != config_len || client.flags != fs_protocol::EOF {
        exit(43)
    }
    config[..n].copy_from_slice(&client.buffer[fs_protocol::HEADER..fs_protocol::HEADER + n]);
    client.close(config_id);
    let mut services = [None; 8];
    let count = services_config::parse(&config[..n], &mut services).unwrap_or_else(|_| exit(43));
    let mut endpoints = [0u32; fs_protocol::MAX_SESSIONS];
    endpoints[0] = fs_control as u32;
    let mut prepared = [(0usize, 0usize); 8];
    for i in 0..count {
        prepared[i] = prepare_source(job, &mut client, services[i].unwrap().path);
    }
    for i in 0..count {
        let spec = services[i].unwrap();
        let (controller, entry) = prepared[i];
        let fs_slot = if spec.filesystem {
            Some(checked(19, [controller, clients[i + 1] as usize, 0, 0, 0, 0]) as u32)
        } else {
            close(clients[i + 1] as usize);
            None
        };
        let endpoint = start_process(controller, entry);
        send(endpoint, &fs_protocol::service_start(fs_slot));
        recv(endpoint, &mut message);
        if service::decode(&message) != Some((service::READY, 0)) {
            exit(3)
        }
        endpoints[i + 1] = endpoint as u32;
    }
    for h in &clients[count + 1..] {
        close(*h as usize);
    }
    close(clients[0] as usize);
    close(job);
    if cfg!(scenario = "init-replay-challenge") {
        send(endpoints[1] as usize, &service::encode(service::PING, 74));
        recv(endpoints[1] as usize, &mut message);
        if service::decode(&message) != Some((service::PONG, 74)) {
            exit(6)
        }
    }
    send(
        bootstrap,
        &protocol::encode_length(protocol::READY, (count + 1) as u64).unwrap(),
    );
    if cfg!(scenario = "init-exit-after-ready") {
        exit(18);
    }
    if cfg!(scenario = "init-close-bootstrap") {
        close(bootstrap);
        exit(19);
    }
    if cfg!(scenario = "init-wrong-channel") {
        let p = pair();
        let mut b = [0u8; 16];
        recv(p[0] as usize, &mut b);
        exit(99);
    }
    let mut waits = [0u32; fs_protocol::MAX_SESSIONS + 1];
    waits[0] = bootstrap as u32;
    waits[1..count + 2].copy_from_slice(&endpoints[..count + 1]);
    loop {
        let index = rt::wait(&waits[..count + 2]);
        if index != 0 {
            exit(5)
        }
        recv(bootstrap, &mut message);
        match service::decode(&message) {
            Some((service::PING, value)) => {
                if cfg!(scenario = "init-replay-challenge") && value == 74 {
                    send(bootstrap, &service::encode(service::PONG, value));
                    continue;
                }
                let ping = message;
                for endpoint in &endpoints[..count + 1] {
                    send(*endpoint as usize, &ping);
                    recv(*endpoint as usize, &mut message);
                    if service::decode(&message) != Some((service::PONG, value)) {
                        exit(4)
                    }
                }
                send(bootstrap, &message);
            }
            Some((service::SHUTDOWN, 0)) => {
                for endpoint in &endpoints[..count + 1] {
                    send(*endpoint as usize, &message);
                }
                checked(0, [0; 6]);
                for endpoint in &endpoints[..count + 1] {
                    close(*endpoint as usize);
                }
                close(bootstrap);
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
