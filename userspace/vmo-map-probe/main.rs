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
#[cfg(not(any(vmo_map_write, vmo_map_execute)))]
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
#[cfg(not(any(vmo_map_write, vmo_map_execute)))]
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
    #[cfg(any(vmo_map_write, vmo_map_execute))]
    {
        let h = syscall6(11, 8192, 1, 0, 0, 0, 0);
        check(h >= 0);
        check(syscall6(14, h as usize, 4096, 0x3000_0000, 4096, 1, 0) == 0);
        close(h as usize);
        close(channel);
        #[cfg(vmo_map_write)]
        map_fault_write();
        #[cfg(vmo_map_execute)]
        {
            let target: unsafe extern "C" fn() = core::mem::transmute(0x3000_0000usize);
            target();
        }
        exit(9);
    }
    #[cfg(not(any(vmo_map_write, vmo_map_execute)))]
    if message[0] == 1 {
        let h = syscall6(11, 8192, 1, 0, 0, 0, 0);
        check(h >= 0);
        let h = h as usize;
        let mut info = [0u32; 2];
        check(syscall6(4, h, info.as_mut_ptr() as usize, 0, 0, 0, 0) == 0 && info == [2, 815]);
        check(syscall6(14, h, 4096, 0x3000_0000, 4096, 3, 0) == 0);
        let ptr = 0x3000_0000 as *mut u64;
        check(ptr.read_volatile() == 0);
        ptr.write_volatile(0x1234_5678);
        let child = syscall6(5, h, 38, 0, 0, 0, 0);
        check(child >= 0);
        send(channel, b"v", &[child as u32]);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"a");
        close(h);
        ptr.write_volatile(0xfeed_beef);
        check(ptr.read_volatile() == 0xfeed_beef);
        send(channel, b"d", &[]);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"u");
        check(syscall6(15, 0x3000_0000, 8192, 0, 0, 0, 0) == -22);
        check(ptr.read_volatile() == 0xfeed_beef);
        // Keep the original frame retained while proving same-VA remapping reads a new VMO.
        let new = syscall6(11, 4096, 1, 0, 0, 0, 0);
        check(new >= 0);
        check(write(new as usize, 0, &0xaabb_ccddu64.to_ne_bytes()) == 0);
        check(syscall6(15, 0x3000_0000, 4096, 0, 0, 0, 0) == 0);
        check(syscall6(14, new as usize, 0, 0x3000_0000, 4096, 3, 0) == 0);
        close(new as usize);
        check(ptr.read_volatile() == 0xaabb_ccdd);
        check(syscall6(15, 0x3000_0000, 4096, 0, 0, 0, 0) == 0);
        check(syscall6(15, 0x3000_0000, 4096, 0, 0, 0, 0) == -22);
        send(channel, b"r", &[]);
    } else {
        check(message[0] == 2);
        let mut received = [0u32; 1];
        recv(channel, &mut message, &mut received, [1, 1]);
        check(message == *b"v");
        let h = received[0] as usize;
        check(syscall6(14, h, 4096, 0x3100_0000, 4096, 3, 0) == -13);
        check(syscall6(14, h, 4096, 0x3100_0000, 4096, 1, 0) == 0);
        check(syscall6(14, h, 4096, 0x3100_0000, 4096, 1, 0) == -17);
        check(syscall6(14, h, 4096, 0x400000, 4096, 1, 0) == -17);
        let ptr = 0x3100_0000 as *const u64;
        check(ptr.read_volatile() == 0x1234_5678);
        close(h);
        send(channel, b"a", &[]);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"d" && ptr.read_volatile() == 0xfeed_beef);
        send(channel, b"u", &[]);
        recv(channel, &mut message, &mut [], [1, 0]);
        check(message == *b"r" && ptr.read_volatile() == 0xfeed_beef);
        check(syscall6(15, 0x3100_0000, 4096, 0, 0, 0, 0) == 0);
    }
    #[cfg(not(any(vmo_map_write, vmo_map_execute)))]
    { close(channel); exit(0) }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(2)
}

#[cfg(all(vmo_map_write, target_arch = "riscv64"))]
core::arch::global_asm!(".section .map_probe,\"ax\"", ".global map_fault_write", "map_fault_write:", ".option push", ".option norvc", "lui t0, 0x30000", "sb zero, 0(t0)", ".option pop");
#[cfg(all(vmo_map_write, target_arch = "x86_64"))]
core::arch::global_asm!(".section .map_probe,\"ax\"", ".global map_fault_write", "map_fault_write:", "mov byte ptr [0x30000000], 1");
#[cfg(vmo_map_write)]
unsafe extern "C" { fn map_fault_write(); }
