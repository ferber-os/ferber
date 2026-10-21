#![no_std]
#![no_main]

#[path = "../../shared/startup.rs"]
mod startup;

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("x86_64.S"));

unsafe extern "C" {
    fn syscall6(number: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize)
        -> isize;
    fn yield_preserved() -> usize;
}

const PATTERN: u64 = 0x12345678;
static RODATA: u64 = PATTERN;
static mut DATA: u64 = PATTERN;
static mut BSS: [u64; 1024] = [0; 1024];

fn exit(code: usize) -> ! {
    unsafe {
        syscall6(1, code, 0, 0, 0, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// # Safety
/// Entry must supply a readable startup record in the retained user stack mapping.
#[no_mangle]
pub unsafe extern "C" fn probe_main(record: *const u8) -> ! {
    if record.is_null() || record.addr() & 15 != 0 {
        exit(1);
    }
    let bytes = core::slice::from_raw_parts(record, startup::SIZE);
    let handle = match startup::Startup::decode(bytes) {
        Ok(startup::Startup {
            bootstrap: Some(handle),
        }) => handle as usize,
        _ => exit(1),
    };
    // Exercise the shared encoder too, so the fixture checks the canonical representation.
    if (startup::Startup {
        bootstrap: Some(handle as u32),
    })
    .encode()
        != bytes
    {
        exit(1);
    }
    if core::ptr::read_volatile(&RODATA) != PATTERN
        || core::ptr::read_volatile(core::ptr::addr_of!(DATA)) != PATTERN
    {
        exit(1);
    }
    let bss = core::ptr::addr_of_mut!(BSS).cast::<u64>();
    for index in 0..1024 {
        if core::ptr::read_volatile(bss.add(index)) != 0 {
            exit(1);
        }
    }
    core::ptr::write_volatile(bss, PATTERN);
    core::ptr::write_volatile(bss.add(1023), PATTERN);
    if syscall6(0x7fff, 0, 0, 0, 0, 0, 0) != -38 {
        exit(1);
    }
    let mut message = [0u8; 16];
    let mut counts = [0u32; 2];
    if syscall6(
        9,
        handle,
        message.as_mut_ptr() as usize,
        message.len(),
        0,
        0,
        counts.as_mut_ptr() as usize,
    ) != 0
        || counts != [8, 0]
        || &message[..8] != b"ferber:1"
    {
        exit(1);
    }
    let reply = b"ready-v1";
    if syscall6(8, handle, reply.as_ptr() as usize, reply.len(), 0, 0, 0) != 0
        || syscall6(2, handle, 0, 0, 0, 0, 0) != 0
        || yield_preserved() != 0
    {
        exit(1);
    }
    if core::ptr::read_volatile(bss) != PATTERN
        || core::ptr::read_volatile(bss.add(1023)) != PATTERN
    {
        exit(1);
    }
    core::ptr::write_volatile(core::ptr::addr_of_mut!(DATA), 0);
    exit(0)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(2)
}
