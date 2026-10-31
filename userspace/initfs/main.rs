#![no_std]
#![no_main]
#[path = "../../shared/fs_protocol.rs"]
mod fs_protocol;
#[path = "../../shared/fs_server.rs"]
mod fs_server;
#[path = "../../shared/initramfs.rs"]
mod initramfs;
#[path = "../common/runtime.rs"]
mod rt;
#[path = "../../shared/service_protocol.rs"]
mod service;
#[path = "../../shared/startup.rs"]
mod startup;
use rt::*;
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("../common/riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("../common/x86_64.S"));
const ARCHIVE_BASE: usize = 0x3000_0000;
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
    let mut b = [0; fs_protocol::FS_START_SIZE];
    let mut handles = [0u32; 1];
    let mut counts = [0u32; 2];
    checked(
        10,
        [
            bootstrap,
            b.as_mut_ptr() as usize,
            b.len(),
            handles.as_mut_ptr() as usize,
            1,
            counts.as_mut_ptr() as usize,
        ],
    );
    if counts != [b.len() as u32, 1] {
        exit(2)
    }
    let (len, slots, n) = fs_protocol::decode_fs_start(&b).unwrap_or_else(|| exit(2));
    if len > initramfs::MAX_SIZE {
        exit(2)
    }
    let size = len.div_ceil(4096) * 4096;
    checked(14, [handles[0] as usize, 0, ARCHIVE_BASE, size, 1, 0]);
    let archive =
        initramfs::Archive::parse(core::slice::from_raw_parts(ARCHIVE_BASE as *const u8, len))
            .unwrap_or_else(|_| exit(2));
    close(handles[0] as usize);
    let mut server = fs_server::Server::new(archive);
    let mut wait = [0u32; fs_protocol::MAX_SESSIONS + 1];
    wait[0] = bootstrap as u32;
    wait[1..n + 1].copy_from_slice(&slots[..n]);
    let mut active = n + 1;
    send(bootstrap, &service::encode(service::READY, 0));
    loop {
        let index = rt::wait(&wait[..active]);
        if index >= active {
            exit(3)
        }
        let h = wait[index] as usize;
        if index == 0 {
            let mut b = [0; service::SIZE];
            recv(h, &mut b);
            match service::decode(&b) {
                Some((service::PING, v)) => send(h, &service::encode(service::PONG, v)),
                Some((service::SHUTDOWN, 0)) => {
                    for slot in &wait[..active] {
                        close(*slot as usize);
                    }
                    checked(15, [ARCHIVE_BASE, size, 0, 0, 0, 0]);
                    exit(0)
                }
                _ => exit(4),
            }
            continue;
        }
        let mut input = [0; fs_protocol::MAX_MESSAGE];
        let mut c = [0u32; 2];
        let result = call(
            9,
            [
                h,
                input.as_mut_ptr() as usize,
                input.len(),
                0,
                0,
                c.as_mut_ptr() as usize,
            ],
        );
        if result != 0 || c[1] != 0 {
            let session = slots[..n].iter().position(|x| *x == wait[index]).unwrap();
            server.disconnect(session);
            close(h);
            wait.copy_within(index + 1..active, index);
            active -= 1;
            continue;
        }
        let session = slots[..n].iter().position(|x| *x == wait[index]).unwrap();
        let mut output = [0; fs_protocol::MAX_MESSAGE];
        let len = server.reply(session, &input[..c[0] as usize], &mut output);
        let delivered = len != 0 && call(8, [h, output.as_ptr() as usize, len, 0, 0, 0]) == 0;
        if !delivered {
            server.disconnect(session);
            close(h);
            wait.copy_within(index + 1..active, index);
            active -= 1;
        }
        if active > 2 {
            wait[1..active].rotate_left(1);
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(99)
}
