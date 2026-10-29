#![no_std]
#![no_main]

#[path = "../../shared/startup.rs"]
mod startup;
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("../common/riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("../common/x86_64.S"));

unsafe extern "C" {
    fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize;
}
fn exit(code: usize) -> ! {
    unsafe { syscall6(1, code, 0, 0, 0, 0, 0); }
    loop { core::hint::spin_loop(); }
}
fn check(condition: bool) { if !condition { exit(1); } }
unsafe fn close(handle: usize) { check(syscall6(2, handle, 0, 0, 0, 0, 0) == 0); }
unsafe fn send(channel: usize, bytes: &[u8], handles: &[u32]) {
    check(syscall6(8, channel, bytes.as_ptr() as usize, bytes.len(),
        handles.as_ptr() as usize, handles.len(), 0) == 0);
}
unsafe fn recv(channel: usize, bytes: &mut [u8], handles: &mut [u32], expected: [u32; 2]) {
    let mut counts = [0u32; 2];
    check(syscall6(10, channel, bytes.as_mut_ptr() as usize, bytes.len(),
        handles.as_mut_ptr() as usize, handles.len(), counts.as_mut_ptr() as usize) == 0);
    check(counts == expected);
}
unsafe fn read(handle: usize, output: &mut [u8]) {
    check(syscall6(12, handle, 4090, output.as_mut_ptr() as usize, output.len(), 0, 0) == 0);
}

/// # Safety
/// The loader supplies a readable startup record on a retained user stack.
#[no_mangle]
pub unsafe extern "C" fn user_main(record: *const u8) -> ! {
    check(!record.is_null() && record.addr() & 15 == 0);
    let channel = match startup::Startup::decode(core::slice::from_raw_parts(record, startup::SIZE)) {
        Ok(startup::Startup { bootstrap: Some(handle) }) => handle as usize,
        _ => exit(1),
    };
    let mut message = [0u8; 1];
    recv(channel, &mut message, &mut [], [1, 0]);
    // Both objects fit the ordinary VMO size policy; the delegated Jobs cannot fund them.
    check(syscall6(11, 16 * 1024 * 1024, 0, 0, 0, 0, 0) == -28);
    if message == [1] {
        let handle = syscall6(11, 8192, 0, 0, 0, 0, 0);
        check(handle >= 0);
        let handle = handle as usize;
        let bytes = b"charged!";
        check(syscall6(13, handle, 4090, bytes.as_ptr() as usize, bytes.len(), 0, 0) == 0);
        let child = syscall6(5, handle, 6, 0, 0, 0, 0);
        check(child >= 0);
        send(channel, b"v", &[child as u32]);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"a");
        close(handle);
        send(channel, b"d", &[]);
    } else {
        check(message == [2]);
        let mut handles = [0u32; 1];
        recv(channel, &mut message, &mut handles, [1, 1]);
        check(message == *b"v");
        let handle = handles[0] as usize;
        let mut bytes = [0u8; 8];
        read(handle, &mut bytes);
        check(&bytes == b"charged!");
        send(channel, b"a", &[]);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"d");
        read(handle, &mut bytes);
        check(&bytes == b"charged!");
        check(syscall6(13, handle, 0, bytes.as_ptr() as usize, bytes.len(), 0, 0) == -13);
        close(handle);
    }
    close(channel);
    exit(0)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { exit(2) }
