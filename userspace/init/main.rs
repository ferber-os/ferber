#![no_std]
#![no_main]

#[path = "../../shared/init_protocol.rs"]
mod protocol;
#[path = "../../shared/startup.rs"]
mod startup;

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("../common/riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("../common/x86_64.S"));

unsafe extern "C" {
    fn syscall6(number: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize)
        -> isize;
}

fn exit(code: usize) -> ! {
    unsafe {
        syscall6(1, code, 0, 0, 0, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// # Safety
/// Entry supplies a readable startup record on the retained user stack.
#[no_mangle]
pub unsafe extern "C" fn user_main(record: *const u8) -> ! {
    if record.is_null() || record.addr() & 15 != 0 {
        exit(1);
    }
    let handle = match startup::Startup::decode(core::slice::from_raw_parts(record, startup::SIZE))
    {
        Ok(startup::Startup {
            bootstrap: Some(handle),
        }) => handle as usize,
        _ => exit(1),
    };
    let mut message = [0u8; protocol::SIZE];
    let mut counts = [0u32; 2];
    if syscall6(
        10,
        handle,
        message.as_mut_ptr() as usize,
        message.len(),
        0,
        0,
        counts.as_mut_ptr() as usize,
    ) != 0
        || counts != [protocol::SIZE as u32, 0]
        || protocol::decode(&message) != Ok(protocol::START)
    {
        exit(1);
    }
    let ready = match protocol::encode(protocol::READY) {
        Ok(bytes) => bytes,
        Err(_) => exit(1),
    };
    if syscall6(8, handle, ready.as_ptr() as usize, ready.len(), 0, 0, 0) != 0 {
        exit(1);
    }
    // No post-startup messages are defined yet; a return is closure, failure, or unsupported work.
    syscall6(
        10,
        handle,
        message.as_mut_ptr() as usize,
        message.len(),
        0,
        0,
        counts.as_mut_ptr() as usize,
    );
    exit(1)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(2)
}
