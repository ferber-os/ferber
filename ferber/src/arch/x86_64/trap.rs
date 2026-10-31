//! x86-64 exception diagnostics and explicitly configured external vectors.

use core::arch::{asm, global_asm};

global_asm!(include_str!("trap.S"));

#[repr(C, packed)]
struct TablePointer {
    limit: u16,
    base: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    flags: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const EMPTY: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        flags: 0,
        offset_mid: 0,
        offset_high: 0,
        reserved: 0,
    };

    fn new(address: usize) -> Self {
        Self {
            offset_low: address as u16,
            selector: 0x08,
            ist: 0,
            flags: 0x8e, // present, ring 0, interrupt gate
            offset_mid: (address >> 16) as u16,
            offset_high: (address >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct Tss {
    reserved0: u32,
    rsp: [u64; 3],
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap: u16,
}
static mut TSS: Tss = Tss {
    reserved0: 0,
    rsp: [0; 3],
    reserved1: 0,
    ist: [0; 7],
    reserved2: 0,
    reserved3: 0,
    iomap: core::mem::size_of::<Tss>() as u16,
};
const _: () = assert!(core::mem::size_of::<Tss>() == 104);
// LTR writes the busy bit, so the descriptor table must be writable.
static mut GDT: [u64; 7] = [
    0,
    0x00af_9b00_0000_ffff,
    0x00cf_9300_0000_ffff,
    0x00cf_f300_0000_ffff,
    0x00af_fb00_0000_ffff,
    0,
    0,
];

pub unsafe fn set_kernel_stack(stack: usize) {
    core::ptr::addr_of_mut!(TSS.rsp)
        .cast::<u64>()
        .write_unaligned(stack as u64);
}
static mut IDT: [IdtEntry; 256] = [IdtEntry::EMPTY; 256];

extern "C" {
    static x86_trap_vectors: [usize; 32];
    fn x86_unexpected_interrupt();
    fn x86_irq_240();
    fn x86_irq_241();
    fn x86_irq_255();
}

pub fn init() {
    let base = (&raw const TSS) as u64;
    unsafe {
        GDT[5] = 103 | ((base & 0xff_ffff) << 16) | (0x89 << 40) | (((base >> 24) & 0xff) << 56);
        GDT[6] = base >> 32;
    }
    let gdt = TablePointer {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: (&raw const GDT) as u64,
    };
    // SAFETY: one bootstrap processor executes this while interrupts are off.
    unsafe {
        asm!("lgdt [{}]", in(reg) &gdt, options(readonly, nostack));
        asm!(
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            out("rax") _,
        );

        asm!("mov ax, 0x28", "ltr ax", out("ax") _, options(nostack));

        let idt = &raw mut IDT;
        let default = IdtEntry::new(x86_unexpected_interrupt as *const () as usize);
        for vector in 0..256 {
            (*idt)[vector] = default;
        }
        for (vector, &address) in x86_trap_vectors.iter().enumerate() {
            (*idt)[vector] = IdtEntry::new(address);
        }
        for (vector, handler) in [
            (super::irq::DEVICE_VECTOR, x86_irq_240 as *const () as usize),
            (super::irq::TIMER_VECTOR, x86_irq_241 as *const () as usize),
            (
                super::irq::SPURIOUS_VECTOR,
                x86_irq_255 as *const () as usize,
            ),
        ] {
            (*idt)[vector as usize] = IdtEntry::new(handler);
        }
        let descriptor = TablePointer {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: idt as u64,
        };
        asm!("lidt [{}]", in(reg) &descriptor, options(readonly, nostack));
    }
}

#[repr(C)]
struct TrapFrame {
    vector: u64,
    error: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
}

#[no_mangle]
extern "C" fn x86_trap_handler(frame: *const TrapFrame) -> ! {
    // SAFETY: assembly passes a pointer to the hardware frame on this stack.
    let frame = unsafe { &*frame };
    if frame.vector == 14 && frame.cs & 3 == 3 {
        let expected = if cfg!(any(
            feature = "user-write-probe",
            feature = "user-vmo-map-write-probe",
            feature = "user-vmo-seal-write-probe"
        )) {
            0x7
        } else if cfg!(any(
            feature = "user-execute-probe",
            feature = "user-vmo-map-execute-probe"
        )) {
            0x15
        } else {
            0x5
        };
        assert_eq!(
            frame.error & 0x1f,
            expected,
            "unexpected user page-fault access"
        );
        let address: usize;
        unsafe {
            asm!("mov {}, cr2", out(reg) address, options(nomem, nostack));
        }
        let cause = if cfg!(any(
            feature = "user-vmo-map-write-probe",
            feature = "user-vmo-map-execute-probe",
            feature = "user-vmo-seal-write-probe"
        )) {
            frame.error as usize
        } else {
            14
        };
        crate::kernel::syscall::user_fault(frame.rip as usize, address, cause);
    }
    panic!(
        "x86 exception vector={} error={:#x} rip={:#x} cs={:#x} rflags={:#x}",
        frame.vector, frame.error, frame.rip, frame.cs, frame.rflags
    );
}
