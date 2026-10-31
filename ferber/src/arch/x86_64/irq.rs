//! Single-CPU, non-preempting external interrupt entry and idle wakeup.
//!
//! A vector may be armed only after its kernel-owned source is contained and
//! configured. Delivery only records work; drivers run outside interrupt context.
// Platform checkpoint: hardware sources are not activated until driver integration.
#![allow(dead_code)]
use core::{
    arch::asm,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

pub const DEVICE_VECTOR: u8 = 0xf0;
pub const TIMER_VECTOR: u8 = 0xf1;
pub const SPURIOUS_VECTOR: u8 = 0xff;
pub const DEVICE_PENDING: u64 = 1;
pub const TIMER_PENDING: u64 = 2;
static APIC: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicU64 = AtomicU64::new(0);
static PENDING: AtomicU64 = AtomicU64::new(0);

unsafe fn read(base: usize, offset: usize) -> u32 {
    ptr::read_volatile((base + offset) as *const u32)
}
unsafe fn write(base: usize, offset: usize, value: u32) {
    ptr::write_volatile((base + offset) as *mut u32, value);
    let _ = read(base, 0x20);
}
/// # Safety
/// Called once on the bootstrap CPU, IF=0, with a permanent supervisor-only
/// uncached mapping of the firmware-discovered local APIC page. No other APIC
/// owner may be active. Device bus mastering remains disabled until DMA and
/// interrupt-remapping publication have completed separately.
pub unsafe fn initialize(virtual_base: usize, physical_base: u64) -> Result<(), &'static str> {
    let mut flags: u64;
    asm!("pushfq","pop {}",out(reg)flags,options(nomem));
    if flags & 512 != 0
        || virtual_base == 0
        || virtual_base & 4095 != 0
        || physical_base & 4095 != 0
        || ENABLED.load(Ordering::Acquire)
    {
        return Err("invalid local APIC initialization state");
    }
    let cpuid = core::arch::x86_64::__cpuid(1);
    if cpuid.edx & (1 << 9) == 0 {
        return Err("local APIC unavailable");
    }
    let msr = super::syscall::read_msr(0x1b);
    if msr & (1 << 10) != 0 || msr & 0xffff_ffff_ffff_f000 != physical_base {
        return Err("unsupported local APIC mode or address");
    }
    super::syscall::write_msr(0x1b, msr | (1 << 11));
    let max_lvt = (read(virtual_base, 0x30) >> 16) & 255;
    if max_lvt < 3 {
        super::syscall::write_msr(0x1b, msr);
        return Err("unsupported local APIC LVT layout");
    }
    // Preserve no firmware interrupt routes. The milestone uses MSI plus HPET,
    // and every legacy PIC source remains masked.
    asm!("out dx, al",in("dx")0x21u16,in("al")0xffu8,options(nomem,nostack));
    asm!("out dx, al",in("dx")0xa1u16,in("al")0xffu8,options(nomem,nostack));
    for offset in [0x320, 0x350, 0x360, 0x370] {
        write(virtual_base, offset, 1 << 16);
    }
    if max_lvt >= 4 {
        write(virtual_base, 0x340, 1 << 16);
    }
    if max_lvt >= 5 {
        write(virtual_base, 0x330, 1 << 16);
    }
    write(virtual_base, 0x80, 0);
    write(virtual_base, 0xf0, (1 << 8) | SPURIOUS_VECTOR as u32);
    APIC.store(virtual_base, Ordering::Release);
    ENABLED.store(true, Ordering::Release);
    Ok(())
}
pub fn configured() -> bool {
    ENABLED.load(Ordering::Acquire)
}
pub fn user_interrupt_flags() -> u64 {
    if configured() {
        512
    } else {
        0
    }
}
pub fn local_apic_id() -> Option<u8> {
    let base = APIC.load(Ordering::Acquire);
    if base == 0 {
        None
    } else {
        Some(unsafe { (read(base, 0x20) >> 24) as u8 })
    }
}
/// # Safety
/// The matching source must already have a kernel-owned interrupt-remapping
/// entry and a masked device configuration. Arm before unmasking the device.
pub unsafe fn arm_source(vector: u8) -> Result<(), &'static str> {
    if !configured() {
        return Err("interrupt controller not configured");
    }
    let bit = source_bit(vector).ok_or("unsupported interrupt vector")?;
    ARMED.fetch_or(bit, Ordering::Release);
    Ok(())
}
/// # Safety
/// Mask and drain the hardware source before disarming. This does not itself
/// revoke a driver's DMA domain or free any backing memory.
pub unsafe fn disarm_source(vector: u8) -> Result<(), &'static str> {
    let bit = source_bit(vector).ok_or("unsupported interrupt vector")?;
    ARMED.fetch_and(!bit, Ordering::AcqRel);
    PENDING.fetch_and(!bit, Ordering::AcqRel);
    Ok(())
}
fn source_bit(vector: u8) -> Option<u64> {
    match vector {
        DEVICE_VECTOR => Some(DEVICE_PENDING),
        TIMER_VECTOR => Some(TIMER_PENDING),
        _ => None,
    }
}
pub fn take_pending() -> u64 {
    PENDING.swap(0, Ordering::AcqRel)
}
/// # Safety
/// The dispatcher must own a live permanent kernel stack, hold no locks, have
/// IF=0, and recheck runnable work and pending events immediately before this
/// call. STI's interrupt shadow makes the following HLT atomic with IRQ entry.
/// No interrupt handler changes the running thread or frees this stack.
pub unsafe fn idle_once() {
    assert!(configured(), "IRQ idle before configuration");
    asm!("sti", "hlt", "cli", options(nomem, nostack));
}

#[repr(C)]
pub struct InterruptFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}
const _: () = {
    assert!(core::mem::offset_of!(InterruptFrame, vector) == 120);
    assert!(core::mem::offset_of!(InterruptFrame, cs) == 144);
    assert!(core::mem::size_of::<InterruptFrame>() == 176);
};
#[no_mangle]
extern "C" fn x86_interrupt_handler(frame: &InterruptFrame) {
    assert!(
        matches!(frame.cs, 0x08 | 0x23),
        "invalid interrupt entry selector"
    );
    let base = APIC.load(Ordering::Acquire);
    assert!(
        base != 0 && configured(),
        "interrupt before local APIC configuration"
    );
    if frame.vector == SPURIOUS_VECTOR as u64 {
        return;
    }
    let bit = source_bit(frame.vector as u8).expect("unexpected interrupt vector");
    assert!(
        ARMED.load(Ordering::Acquire) & bit != 0,
        "unconfigured interrupt source"
    );
    PENDING.fetch_or(bit, Ordering::Release);
    unsafe {
        write(base, 0xb0, 0);
    }
}
