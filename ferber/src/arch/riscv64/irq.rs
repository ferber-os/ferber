//! Single-hart console interrupts. Top halves only mask, acknowledge and latch bits.
#![allow(dead_code)]
use super::device_tree::ConsolePlatform;
use core::sync::atomic::{AtomicUsize, Ordering};
const EXTERNAL: usize = 1 << 9;
const TIMER: usize = 1 << 5;
const UART_PENDING: usize = 1;
const TIMER_PENDING: usize = 2;
const UNEXPECTED: usize = 4;
#[no_mangle]
pub static RISCV_CONSOLE_SIE: AtomicUsize = AtomicUsize::new(0);
static PLIC: AtomicUsize = AtomicUsize::new(0);
static CONTEXT: AtomicUsize = AtomicUsize::new(0);
static UART_IRQ: AtomicUsize = AtomicUsize::new(0);
static PENDING: AtomicUsize = AtomicUsize::new(0);
static FREQUENCY: AtomicUsize = AtomicUsize::new(0);
#[derive(Clone, Copy)]
pub struct Pending {
    pub external: bool,
    pub timer: bool,
    pub overflow: bool,
}
unsafe fn read(base: usize, offset: usize) -> u32 {
    core::ptr::read_volatile((base + offset) as *const u32)
}
unsafe fn write(base: usize, offset: usize, value: u32) {
    core::ptr::write_volatile((base + offset) as *mut u32, value)
}
fn context_offset() -> usize {
    0x20_0000 + CONTEXT.load(Ordering::Relaxed) * 0x1000
}
fn enable_offset() -> usize {
    0x2000 + CONTEXT.load(Ordering::Relaxed) * 0x80
}
fn enable_bits(bits: usize) {
    RISCV_CONSOLE_SIE.fetch_or(bits, Ordering::Release);
    // Only sie changes here; kernel SIE remains disabled until the controlled idle window.
    unsafe {
        core::arch::asm!("csrs sie, {}",in(reg)bits,options(nostack));
    }
}
fn disable_bits(bits: usize) {
    unsafe {
        core::arch::asm!("csrc sie, {}",in(reg)bits,options(nostack));
    }
    RISCV_CONSOLE_SIE.fetch_and(!bits, Ordering::Release);
}
/// # Safety
/// Call once on the boot hart with SIE clear. `mapped_plic` must map the complete
/// discovered PLIC region supervisor RW, uncached, with no aliases of differing type.
/// The caller retains its mapping until all console IRQ authority is destroyed.
pub unsafe fn configure(
    platform: ConsolePlatform,
    _mapped_uart: usize,
    mapped_plic: usize,
) -> Result<(), &'static str> {
    if mapped_plic == 0
        || mapped_plic & 3 != 0
        || PLIC.load(Ordering::Acquire) != 0
        || platform.uart_irq == 0
        || platform.uart_irq > 1023
    {
        return Err("invalid console IRQ configuration");
    }
    let status: usize;
    core::arch::asm!("csrr {}, sstatus",out(reg)status,options(nostack));
    if status & 2 != 0 {
        return Err("console IRQ configuration with interrupts enabled");
    }
    let context = platform.plic_context as usize;
    let end = 0x20_0008usize
        .checked_add(context.checked_mul(0x1000).ok_or("PLIC context overflow")?)
        .ok_or("PLIC context overflow")?;
    if end as u64
        > platform
            .plic
            .end
            .checked_sub(platform.plic.start)
            .ok_or("invalid PLIC extent")?
        || mapped_plic.checked_add(end).is_none()
        || platform.timebase_hz == 0
    {
        return Err("invalid PLIC context");
    }
    CONTEXT.store(context, Ordering::Relaxed);
    UART_IRQ.store(platform.uart_irq as usize, Ordering::Relaxed);
    FREQUENCY.store(platform.timebase_hz as usize, Ordering::Relaxed);
    // No source is enabled until the user driver explicitly arms its route.
    for word in 0..32 {
        write(mapped_plic, enable_offset() + word * 4, 0);
    }
    write(mapped_plic, context_offset(), 0);
    write(mapped_plic, platform.uart_irq as usize * 4, 1);
    core::arch::asm!("fence iorw, iorw", options(nostack));
    PLIC.store(mapped_plic, Ordering::Release);
    Ok(())
}
pub fn configured() -> bool {
    PLIC.load(Ordering::Acquire) != 0
}
/// # Safety
/// Call on the boot hart with normal kernel interrupts disabled. The console
/// driver must have drained its device and retained the corresponding authority.
pub unsafe fn arm() {
    let base = PLIC.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let irq = UART_IRQ.load(Ordering::Relaxed);
    let offset = enable_offset() + irq / 32 * 4;
    write(base, offset, read(base, offset) | (1u32 << (irq % 32)));
    core::arch::asm!("fence iorw, iorw", options(nostack));
    enable_bits(EXTERNAL);
}
/// # Safety
/// Same boot-hart and interrupt-state requirements as `arm`.
pub unsafe fn disarm() {
    disable_bits(EXTERNAL);
    let base = PLIC.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let irq = UART_IRQ.load(Ordering::Relaxed);
    let offset = enable_offset() + irq / 32 * 4;
    write(base, offset, read(base, offset) & !(1u32 << (irq % 32)));
    core::arch::asm!("fence iorw, iorw", options(nostack));
}
/// Current monotonic platform ticks, not wall-clock time.
pub fn now() -> u64 {
    let ticks: u64;
    unsafe {
        core::arch::asm!("rdtime {}",out(reg)ticks,options(nostack));
    }
    ticks
}
pub fn frequency() -> usize {
    FREQUENCY.load(Ordering::Acquire)
}
/// Arm a supervisor timer without changing scheduling policy.
/// # Safety
/// Call on the boot hart with normal kernel interrupts disabled and live timer authority.
pub unsafe fn arm_deadline(deadline: u64) -> Result<(), &'static str> {
    if !configured() {
        return Err("console timer not configured");
    }
    let error: isize;
    core::arch::asm!("ecall",inlateout("a0") deadline as usize=>error,inlateout("a1")0usize=>_,in("a6")0usize,in("a7")0x5449_4d45usize,lateout("a2")_,lateout("a3")_,lateout("a4")_,lateout("a5")_,options(nostack));
    if error != 0 {
        return Err("SBI TIME rejected deadline");
    }
    enable_bits(TIMER);
    Ok(())
}
pub fn disarm_deadline() {
    disable_bits(TIMER);
}
pub fn take_pending() -> Pending {
    let bits = PENDING.swap(0, Ordering::AcqRel);
    Pending {
        external: bits & UART_PENDING != 0,
        timer: bits & TIMER_PENDING != 0,
        overflow: bits & UNEXPECTED != 0,
    }
}
pub fn pending() -> bool {
    PENDING.load(Ordering::Acquire) != 0
}
/// Handles only the configured console routes; preserves every interrupted register.
/// # Safety
/// Called from the trap entry with SIE clear, a trusted stack, and retained mappings.
pub unsafe fn capture(cause: usize) -> bool {
    let base = PLIC.load(Ordering::Acquire);
    if base == 0 {
        return false;
    }
    if cause == 5 {
        disable_bits(TIMER);
        PENDING.fetch_or(TIMER_PENDING, Ordering::Release);
        return true;
    }
    if cause != 9 {
        return false;
    }
    let claimed = read(base, context_offset() + 4) as usize;
    if claimed == 0 {
        return true;
    }
    // Mask before completing a level-triggered source, leaving register draining to its owner.
    if claimed > 1023 {
        disable_bits(EXTERNAL);
        PENDING.fetch_or(UNEXPECTED, Ordering::Release);
        return true;
    }
    let offset = enable_offset() + claimed / 32 * 4;
    write(base, offset, read(base, offset) & !(1u32 << (claimed % 32)));
    core::arch::asm!("fence iorw, iorw", options(nostack));
    write(base, context_offset() + 4, claimed as u32);
    core::arch::asm!("fence iorw, iorw", options(nostack));
    disable_bits(EXTERNAL);
    PENDING.fetch_or(
        if claimed == UART_IRQ.load(Ordering::Relaxed) {
            UART_PENDING
        } else {
            UNEXPECTED
        },
        Ordering::Release,
    );
    true
}
/// Sleep only on the retained dispatcher stack with no locks held.
/// # Safety
/// The caller must atomically reconcile scheduler readiness before calling, retain
/// live IRQ source leases, and enter with SIE clear. IRQ handlers never schedule.
pub unsafe fn idle_wait() {
    if !configured() || pending() || RISCV_CONSOLE_SIE.load(Ordering::Acquire) == 0 {
        return;
    }
    // WFI must observe pending interrupts even while global SIE is clear. Rechecking
    // SIP closes the arrival-before-WFI window without exposing ordinary kernel code.
    let sip: usize;
    core::arch::asm!("csrr {}, sip",out(reg)sip,options(nostack));
    if sip & RISCV_CONSOLE_SIE.load(Ordering::Acquire) != 0 {
        core::arch::asm!("csrsi sstatus, 2", "csrci sstatus, 2", options(nostack));
    } else {
        core::arch::asm!(
            "wfi",
            "csrsi sstatus, 2",
            "csrci sstatus, 2",
            options(nostack)
        );
    }
}
