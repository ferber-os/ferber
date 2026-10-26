#![no_std]
#![no_main]

#[path = "../../shared/startup.rs"]
mod startup;
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("../common/riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("../common/x86_64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(
    ".section .text.fragments,\"ax\"", ".option push", ".option norvc", ".balign 4",
    ".global fragment_a, fragment_a_end, fragment_b, fragment_b_end",
    "fragment_a:", "addi a0, zero, 17", "ret", "fragment_a_end:",
    "fragment_b:", "addi a0, zero, 42", "ret", "fragment_b_end:", ".option pop",
);
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".section .text.fragments,\"ax\"",
    ".global fragment_a, fragment_a_end, fragment_b, fragment_b_end",
    "fragment_a:", "mov eax, 17", "ret", "fragment_a_end:",
    "fragment_b:", "mov eax, 42", "ret", "fragment_b_end:",
);

unsafe extern "C" {
    fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize;
    static fragment_a: u8;
    static fragment_a_end: u8;
    #[cfg(not(vmo_seal_write))]
    static fragment_b: u8;
    #[cfg(not(vmo_seal_write))]
    static fragment_b_end: u8;
}

const BASE: usize = 0x3000_0000;
#[cfg(not(vmo_seal_write))]
const PEER_BASE: usize = 0x3100_0000;

fn exit(code: usize) -> ! {
    unsafe { syscall6(1, code, 0, 0, 0, 0, 0); }
    loop { core::hint::spin_loop(); }
}
fn check(value: bool) { if !value { exit(1); } }
unsafe fn close(h: usize) { check(syscall6(2, h, 0, 0, 0, 0, 0) == 0); }
unsafe fn write(h: usize, bytes: &[u8]) -> isize {
    syscall6(13, h, 0, bytes.as_ptr() as usize, bytes.len(), 0, 0)
}
unsafe fn seal(h: usize) -> isize { syscall6(16, h, 0, 0, 0, 0, 0) }
unsafe fn map(h: usize, base: usize, permissions: usize) -> isize {
    syscall6(14, h, 0, base, 4096, permissions, 0)
}
#[cfg(not(vmo_seal_write))]
unsafe fn unmap(base: usize) { check(syscall6(15, base, 4096, 0, 0, 0, 0) == 0); }
unsafe fn execute(base: usize) -> usize {
    let function: unsafe extern "C" fn() -> usize = core::mem::transmute(base);
    function()
}
unsafe fn fragment(start: *const u8, end: *const u8) -> &'static [u8] {
    let length = end.addr() - start.addr();
    check(length != 0 && length <= 4096);
    core::slice::from_raw_parts(start, length)
}
#[cfg(not(vmo_seal_write))]
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
#[cfg(not(vmo_seal_write))]
unsafe fn expect(channel: usize, byte: u8) {
    let mut message = [0];
    recv(channel, &mut message, &mut [], [1, 0]);
    check(message == [byte]);
}

/// # Safety
/// The loader supplies a retained, readable startup record on the user stack.
#[no_mangle]
pub unsafe extern "C" fn user_main(record: *const u8) -> ! {
    check(!record.is_null() && record.addr() & 15 == 0);
    let channel = match startup::Startup::decode(core::slice::from_raw_parts(record, startup::SIZE)) {
        Ok(startup::Startup { bootstrap: Some(handle) }) => handle as usize,
        _ => exit(1),
    };
    let mut role = [0];
    recv(channel, &mut role, &mut [], [1, 0]);
    #[cfg(vmo_seal_write)]
    {
        let h = syscall6(11, 4096, 3, 0, 0, 0, 0);
        check(h >= 0);
        let h = h as usize;
        check(write(h, fragment(&raw const fragment_a, &raw const fragment_a_end)) == 0);
        check(seal(h) == 0 && map(h, BASE, 5) == 0);
        check(execute(BASE) == 17);
        close(h);
        close(channel);
        seal_fault_write();
        exit(9);
    }
    #[cfg(not(vmo_seal_write))]
    {
        if role == [1] { producer(channel); }
        else { check(role == [2]); consumer(channel); }
        close(channel);
        exit(0);
    }
}

#[cfg(not(vmo_seal_write))]
unsafe fn producer(channel: usize) {
    let h = syscall6(11, 4096, 3, 0, 0, 0, 0);
    check(h >= 0);
    let h = h as usize;
    let mut info = [0u32; 2];
    check(syscall6(4, h, info.as_mut_ptr() as usize, 0, 0, 0, 0) == 0 && info == [2, 831]);
    check(write(h, fragment(&raw const fragment_a, &raw const fragment_a_end)) == 0);
    check(map(h, BASE, 5) == -13);
    // Retain a managing alias so revocation cannot disguise a still-writable mapping.
    let child = syscall6(5, h, 559, 0, 0, 0, 0);
    check(child >= 0);
    let child = child as usize;
    let sent = syscall6(3, child, 0, 0, 0, 0, 0);
    check(sent >= 0);
    send(channel, b"w", &[sent as u32]);
    expect(channel, b'm');
    check(seal(h) == -16);
    check(syscall6(6, child, 0, 0, 0, 0, 0) == 0);
    check(seal(h) == -16);
    close(child);
    send(channel, b"u", &[]);
    expect(channel, b'a');
    check(seal(h) == 0 && seal(h) == 0);
    check(write(h, &[0xff]) == -13 && write(h, &[]) == -13);
    check(map(h, BASE, 3) == -13);
    check(map(h, BASE, 5) == 0);
    let first = execute(BASE);
    check(first == 17);
    let executable = syscall6(5, h, 54, 0, 0, 0, 0);
    check(executable >= 0);
    send(channel, b"e", &[executable as u32]);
    expect(channel, b'v');
    check(syscall6(6, h, 0, 0, 0, 0, 0) == 0);
    check(execute(BASE) == first);
    close(h);
    send(channel, b"r", &[]);
    expect(channel, b'a');
    // Hold A's mapping until B exists, then replace the same VA with different code.
    let b = syscall6(11, 4096, 3, 0, 0, 0, 0);
    check(b >= 0);
    let b = b as usize;
    check(write(b, fragment(&raw const fragment_b, &raw const fragment_b_end)) == 0);
    check(seal(b) == 0);
    unmap(BASE);
    check(map(b, BASE, 5) == 0);
    close(b);
    let second = execute(BASE);
    check(second == 42);
    unmap(BASE);
    let mut report = [0u8; 17];
    report[0] = b'P';
    report[1..9].copy_from_slice(&(first as u64).to_le_bytes());
    report[9..17].copy_from_slice(&(second as u64).to_le_bytes());
    send(channel, &report, &[]);
}

#[cfg(not(vmo_seal_write))]
unsafe fn consumer(channel: usize) {
    let mut message = [0];
    let mut handle = [0];
    recv(channel, &mut message, &mut handle, [1, 1]);
    check(message == *b"w");
    let h = handle[0] as usize;
    check(map(h, PEER_BASE, 3) == 0);
    close(h);
    send(channel, b"m", &[]);
    expect(channel, b'u');
    unmap(PEER_BASE);
    send(channel, b"a", &[]);
    recv(channel, &mut message, &mut handle, [1, 1]);
    check(message == *b"e");
    let h = handle[0] as usize;
    check(seal(h) == -13 && map(h, PEER_BASE, 3) == -13);
    check(map(h, PEER_BASE, 5) == 0);
    let first = execute(PEER_BASE);
    check(first == 17);
    close(h);
    check(execute(PEER_BASE) == first);
    send(channel, b"v", &[]);
    expect(channel, b'r');
    let second = execute(PEER_BASE);
    check(second == 17);
    unmap(PEER_BASE);
    send(channel, b"a", &[]);
    let mut report = [0u8; 17];
    report[0] = b'C';
    report[1..9].copy_from_slice(&(first as u64).to_le_bytes());
    report[9..17].copy_from_slice(&(second as u64).to_le_bytes());
    send(channel, &report, &[]);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { exit(2) }

#[cfg(all(vmo_seal_write, target_arch = "riscv64"))]
core::arch::global_asm!(
    ".section .seal_probe,\"ax\"", ".global seal_fault_write", "seal_fault_write:",
    ".option push", ".option norvc", "lui t0, 0x30000", "sb zero, 0(t0)", ".option pop",
);
#[cfg(all(vmo_seal_write, target_arch = "x86_64"))]
core::arch::global_asm!(
    ".section .seal_probe,\"ax\"", ".global seal_fault_write", "seal_fault_write:",
    "mov byte ptr [0x30000000], 1",
);
#[cfg(vmo_seal_write)]
unsafe extern "C" { fn seal_fault_write(); }
