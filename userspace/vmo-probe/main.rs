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
    unsafe {
        syscall6(1, code, 0, 0, 0, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}
fn check(value: bool) {
    if !value {
        exit(1);
    }
}
unsafe fn close(h: usize) {
    check(syscall6(2, h, 0, 0, 0, 0, 0) == 0);
}
unsafe fn send(channel: usize, message: &[u8], handles: &[u32]) {
    check(
        syscall6(
            8,
            channel,
            message.as_ptr() as usize,
            message.len(),
            handles.as_ptr() as usize,
            handles.len(),
            0,
        ) == 0,
    );
}
unsafe fn recv(channel: usize, bytes: &mut [u8], handles: &mut [u32], expected: [u32; 2]) {
    let mut counts = [0u32; 2];
    check(
        syscall6(
            10,
            channel,
            bytes.as_mut_ptr() as usize,
            bytes.len(),
            handles.as_mut_ptr() as usize,
            handles.len(),
            counts.as_mut_ptr() as usize,
        ) == 0,
    );
    check(counts == expected);
}
unsafe fn read(h: usize, offset: usize, bytes: &mut [u8]) -> isize {
    syscall6(
        12,
        h,
        offset,
        bytes.as_mut_ptr() as usize,
        bytes.len(),
        0,
        0,
    )
}
unsafe fn write(h: usize, offset: usize, bytes: &[u8]) -> isize {
    syscall6(13, h, offset, bytes.as_ptr() as usize, bytes.len(), 0, 0)
}
/// # Safety
/// The loader supplies a readable startup record on the retained user stack.
#[no_mangle]
pub unsafe extern "C" fn user_main(record: *const u8) -> ! {
    check(!record.is_null() && record.addr() & 15 == 0);
    let startup = startup::Startup::decode(core::slice::from_raw_parts(record, startup::SIZE));
    let channel = match startup {
        Ok(startup::Startup { bootstrap: Some(h) }) => h as usize,
        _ => exit(1),
    };
    let mut message = [0u8; 1];
    recv(channel, &mut message, &mut [], [1, 0]);
    if message[0] == 1 {
        check(syscall6(11, 0, 0, 0, 0, 0, 0) == -22);
        check(syscall6(11, 1, 1, 0, 0, 0, 0) == -22);
        check(syscall6(11, 16 * 1024 * 1024 + 1, 0, 0, 0, 0, 0) == -90);
        let result = syscall6(11, 4097, 0, 0, 0, 0, 0);
        check(result >= 0);
        let h = result as usize;
        let mut info = [0u32; 2];
        check(syscall6(4, h, info.as_mut_ptr() as usize, 0, 0, 0, 0) == 0);
        check(info == [2, 783]); // READ | WRITE | DUPLICATE | TRANSFER | INSPECT | MANAGE.
        check(syscall6(12, h | (1usize << 32), 0, 0, 0, 0, 0) == -9);
        let mut bytes = [1u8; 17];
        check(read(h, 4080, &mut bytes) == 0 && bytes == [0; 17]);
        check(write(h, 4090, b"ferber!") == 0);
        check(read(h, 4090, &mut bytes[..7]) == 0 && &bytes[..7] == b"ferber!");
        check(read(h, 4097, &mut bytes[..1]) == -22);
        check(syscall6(12, h, 4097, usize::MAX, 0, 0, 0) == 0);
        check(syscall6(12, h, 4098, usize::MAX, 0, 0, 0) == -22);
        check(syscall6(12, h, usize::MAX, 0, 1, 0, 0) == -22);
        check(syscall6(12, h, 0, 0, 65537, 0, 0) == -90);
        check(syscall6(12, h, 0, 0, 1, 0, 0) == -14);
        let alias = syscall6(3, h, 0, 0, 0, 0, 0);
        check(alias >= 0);
        // READ | TRANSFER: the receiver cannot mutate this derived child.
        let child = syscall6(5, alias as usize, 6, 0, 0, 0, 0);
        check(child >= 0);
        close(alias as usize);
        send(channel, b"v", &[child as u32]);
        check(syscall6(12, child as usize, 0, 0, 0, 0, 0) == -9);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"a");
        check(write(h, 4090, b"shared!") == 0);
        send(channel, b"d", &[]);
        close(h);
    } else {
        check(message[0] == 2);
        let mut received = [0u32; 1];
        recv(channel, &mut message, &mut received, [1, 1]);
        check(message == *b"v");
        let h = received[0] as usize;
        let mut bytes = [0u8; 7];
        check(read(h, 4090, &mut bytes) == 0 && &bytes == b"ferber!");
        check(write(h, 4090, b"denied!") == -13);
        send(channel, b"a", &[]);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"d");
        check(read(h, 4090, &mut bytes) == 0 && &bytes == b"shared!");
        close(h);
    }
    close(channel);
    exit(0)
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(2)
}
