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
