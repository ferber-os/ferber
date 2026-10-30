#![no_std]
#![no_main]
#[path = "../../shared/service_protocol.rs"]
mod protocol;
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
#[path = "../common/runtime.rs"]
mod rt;
use rt::{call, exit};
#[no_mangle]
pub unsafe extern "C" fn user_main(record: *const u8) -> ! {
    if record.is_null() || record.addr() & 15 != 0 {
        exit(1)
    }
    let handle = match startup::Startup::decode(core::slice::from_raw_parts(record, startup::SIZE))
    {
        Ok(startup::Startup { bootstrap: Some(h) }) => h as usize,
        _ => exit(1),
    };
    let mut start = [0; 16];
    rt::recv(handle, &mut start);
    let fs = fs_protocol::decode_service_start(&start).unwrap_or_else(|| exit(2));
    if let Some(slot) = fs {
        let mut client = fs_client::Client::new(slot as usize);
        let (id, len) = client.open("etc/services.conf", fs_protocol::FILE);
        client.stat(id);
        if len == 0 || client.read(id, 0, len.min(fs_protocol::CHUNK)) == 0 {
            exit(2)
        }
        client.close(id);
        let (root, _) = client.open("/", fs_protocol::DIRECTORY);
        if client.list(root, 0).is_none() {
            exit(2)
        }
        client.close(root);
        if cfg!(fs_attack = "malformed") {
            rt::send(slot as usize, &[0]);
            let b = &mut client.buffer;
            let mut counts = [0u32; 2];
            if call(
                10,
                [
                    slot as usize,
                    b.as_mut_ptr() as usize,
                    b.len(),
                    0,
                    0,
                    counts.as_mut_ptr() as usize,
                ],
            ) != -32
            {
                exit(6);
            }
        }
        if cfg!(fs_attack = "flood") {
            let mut b = [0u8; fs_protocol::HEADER + 1];
            fs_protocol::encode(
                fs_protocol::Header {
                    op: fs_protocol::OPEN,
                    request: 1,
                    status: 0,
                    flags: fs_protocol::DIRECTORY,
                    object: 0,
                },
                &mut b,
            );
            b[fs_protocol::HEADER] = b'/';
            for _ in 0..65 {
                let sent = call(8, [slot as usize, b.as_ptr() as usize, b.len(), 0, 0, 0]);
                if sent == -32 {
                    break;
                }
                if sent != 0 {
                    exit(6);
                }
                rt::checked(0, [0; 6]);
            }
            // Let the server exhaust its reply queue before this client drains it.
            for _ in 0..16 {
                rt::checked(0, [0; 6]);
            }
            let response = &mut client.buffer;
            let mut counts = [0u32; 2];
            for i in 0..65 {
                let r = call(
                    10,
                    [
                        slot as usize,
                        response.as_mut_ptr() as usize,
                        response.len(),
                        0,
                        0,
                        counts.as_mut_ptr() as usize,
                    ],
                );
                if r == -32 {
                    break;
                }
                if r != 0 || i == 64 {
                    exit(6);
                }
            }
        }
        rt::close(slot as usize);
    }
    let ready = protocol::encode(protocol::READY, 0);
    if call(8, [handle, ready.as_ptr() as usize, ready.len(), 0, 0, 0]) != 0 {
        exit(2)
    }
    loop {
        let mut b = [0; protocol::SIZE];
        let mut counts = [0u32; 2];
        if call(
            10,
            [
                handle,
                b.as_mut_ptr() as usize,
                b.len(),
                0,
                0,
                counts.as_mut_ptr() as usize,
            ],
        ) != 0
            || counts != [protocol::SIZE as u32, 0]
        {
            exit(3)
        }
        match protocol::decode(&b) {
            Some((protocol::PING, value)) => {
                let pong = protocol::encode(protocol::PONG, value);
                if call(8, [handle, pong.as_ptr() as usize, pong.len(), 0, 0, 0]) != 0 {
                    exit(4)
                }
            }
            Some((protocol::SHUTDOWN, 0)) => {
                call(2, [handle, 0, 0, 0, 0, 0]);
                exit(0)
            }
            _ => exit(5),
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(99)
}
