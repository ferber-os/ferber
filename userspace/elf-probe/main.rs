#![no_std]
#![no_main]

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("x86_64.S"));

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
