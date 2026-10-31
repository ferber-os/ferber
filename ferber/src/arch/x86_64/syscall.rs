//! Single-CPU syscall entry. Kernel syscalls always execute with IF masked.

use core::arch::{asm, global_asm};

global_asm!(
    include_str!("syscall.S"),
    write_probe = const cfg!(feature = "user-write-probe") as usize,
    user_pc = const crate::kernel::syscall::USER_PC,
    sys_chan_create = const crate::kernel::syscall::SYS_CHAN_CREATE,
    sys_chan_send = const crate::kernel::syscall::SYS_CHAN_SEND,
    sys_chan_recv = const crate::kernel::syscall::SYS_CHAN_RECV,
    sys_chan_recv_wait = const crate::kernel::syscall::SYS_CHAN_RECV_WAIT,
    user_stack = const crate::kernel::syscall::USER_STACK,
    stack_size = const crate::kernel::syscall::STACK_SIZE,
    sys_yield = const crate::kernel::syscall::SYS_YIELD,
    sys_exit = const crate::kernel::syscall::SYS_EXIT,
    sys_handle_close = const crate::kernel::syscall::SYS_HANDLE_CLOSE,
    sys_handle_duplicate = const crate::kernel::syscall::SYS_HANDLE_DUPLICATE,
    sys_cap_inspect = const crate::kernel::syscall::SYS_CAP_INSPECT,
    sys_cap_attenuate = const crate::kernel::syscall::SYS_CAP_ATTENUATE,
    sys_cap_revoke = const crate::kernel::syscall::SYS_CAP_REVOKE,
);

#[repr(C)]
pub struct SyscallFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r9: u64,
    pub r8: u64,
    pub r10: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

const _: () = {
    assert!(core::mem::size_of::<SyscallFrame>() == 144);
    assert!(core::mem::offset_of!(SyscallFrame, rax) == 96);
    assert!(core::mem::offset_of!(SyscallFrame, rip) == 104);
    assert!(core::mem::offset_of!(SyscallFrame, rsp) == 128);
};

#[repr(C)]
struct CpuState {
    kernel_sp: usize,
    user_sp: usize,
}
static mut CPU: CpuState = CpuState {
    kernel_sp: 0,
    user_sp: 0,
};

pub unsafe fn read_msr(index: u32) -> u64 {
    let low: u32;
    let high: u32;
    asm!("rdmsr", in("ecx") index, out("eax") low, out("edx") high, options(nostack));
    (high as u64) << 32 | low as u64
}

pub unsafe fn write_msr(index: u32, value: u64) {
    asm!("wrmsr", in("ecx") index, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nostack));
}

pub fn init() {
    // SAFETY: called once on the bootstrap CPU before any user execution.
    unsafe {
        let max = core::arch::x86_64::__cpuid(0x8000_0000).eax;
        assert!(
            max >= 0x8000_0001
                && core::arch::x86_64::__cpuid(0x8000_0001).edx & ((1 << 20) | (1 << 11))
                    == ((1 << 20) | (1 << 11)),
            "x86 privilege boundary requires NX and SYSCALL"
        );
        // Unsaved user bases and FP/SIMD registers must not pass between tasks.
        let mut cr0: u64;
        let mut cr4: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        cr0 |= 1 << 3; // TS: unsupported x87/MMX/SIMD operations trap.
        cr4 &= !((1 << 16) | (1 << 22)); // No unsaved user FS/GS or PKRU state.
        asm!("mov cr0, {}", in(reg) cr0, options(nostack));
        asm!("mov cr4, {}", in(reg) cr4, options(nostack));
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        assert_ne!(cr0 & (1 << 3), 0, "integer-only user state requires CR0.TS");
        assert_eq!(
            cr4 & (1 << 16),
            0,
            "unsaved user FS/GS bases must be disabled"
        );
        assert_eq!(
            cr4 & (1 << 22),
            0,
            "unsaved user PKRU state must be disabled"
        );
        write_msr(0xc000_0080, read_msr(0xc000_0080) | 1 | (1 << 11));
        write_msr(0xc000_0081, (0x10u64 << 48) | (0x08u64 << 32));
        write_msr(0xc000_0082, x86_syscall_entry as *const () as u64);
        write_msr(
            0xc000_0084,
            (1 << 9) | (1 << 10) | (1 << 8) | (1 << 14) | (1 << 18),
        );
    }
}

unsafe extern "C" {
    fn x86_syscall_entry();
    fn x86_enter_user(pc: usize, sp: usize, flags: u64) -> !;
    static x86_user_probe_start: u8;
    static x86_user_probe_end: u8;
    static x86_cooperative_probe_start: u8;
    static x86_cooperative_probe_end: u8;
    static x86_isolation_start: u8;
    static x86_isolation_end: u8;
    static x86_isolation_fault: u8;
}

#[no_mangle]
extern "C" fn x86_syscall_handler(frame: &mut SyscallFrame, entry_flags: u64) {
    assert_eq!(
        entry_flags & ((1 << 9) | (1 << 10) | (1 << 8) | (1 << 14) | (1 << 18)),
        0,
        "unsafe syscall entry flags"
    );
    crate::kernel::syscall::observe_x86_flags(frame.rax as usize, frame.rflags);
    assert!(
        frame.cs == 0x23 && frame.ss == 0x1b,
        "invalid user selectors"
    );
    frame.rax = crate::kernel::syscall::dispatch(
        frame.rax as usize,
        [
            frame.rdi as usize,
            frame.rsi as usize,
            frame.rdx as usize,
            frame.r10 as usize,
            frame.r8 as usize,
            frame.r9 as usize,
        ],
    ) as u64;
    assert!(
        frame.rip != 0
            && frame.rip <= 0x7fff_ffff_ffff
            && frame.rsp != 0
            && frame.rsp <= 0x7fff_ffff_ffff,
        "invalid user return addresses"
    );
    frame.rflags = (frame.rflags & 0x8d5) | 2 | super::irq::user_interrupt_flags();
}

/// Enter ring 3. Does not return.
///
/// # Safety
/// The active root must retain the kernel and executable user mappings; the
/// supplied user and kernel stacks must remain allocated and correctly aligned.
pub unsafe fn drop_to_user(user_pc: usize, user_sp: usize, kernel_sp: usize) -> ! {
    bind_user_kernel_stack(kernel_sp);
    assert_eq!(user_sp & 15, 0);
    CPU.user_sp = 0;
    asm!("xor eax, eax", "mov fs, ax", "mov gs, ax", out("eax") _, options(nostack));
    write_msr(0xc000_0100, 0);
    write_msr(0xc000_0101, (&raw mut CPU) as u64);
    write_msr(0xc000_0102, 0);
    x86_enter_user(user_pc, user_sp, 2 | super::irq::user_interrupt_flags())
}

/// Bind both trusted entry paths to the selected task's retained kernel stack.
///
/// # Safety
/// Called on the sole active CPU with interrupts masked and a live supervisor stack.
pub unsafe fn bind_user_kernel_stack(kernel_sp: usize) {
    assert_eq!(kernel_sp & 15, 0);
    super::trap::set_kernel_stack(kernel_sp);
    CPU.kernel_sp = kernel_sp;
}

pub fn cooperative_probe() -> &'static [u8] {
    unsafe {
        core::slice::from_raw_parts(
            &raw const x86_cooperative_probe_start,
            (&raw const x86_cooperative_probe_end as usize)
                - (&raw const x86_cooperative_probe_start as usize),
        )
    }
}

pub fn user_probe() -> &'static [u8] {
    unsafe {
        core::slice::from_raw_parts(
            &raw const x86_user_probe_start,
            (&raw const x86_user_probe_end as usize) - (&raw const x86_user_probe_start as usize),
        )
    }
}
pub fn isolation_probe() -> &'static [u8] {
    unsafe {
        core::slice::from_raw_parts(
            &raw const x86_isolation_start,
            (&raw const x86_isolation_end as usize) - (&raw const x86_isolation_start as usize),
        )
    }
}
pub fn isolation_fault_offset() -> usize {
    (&raw const x86_isolation_fault as usize) - (&raw const x86_isolation_start as usize)
}
pub fn isolation_address() -> usize {
    if cfg!(any(
        feature = "user-write-probe",
        feature = "user-execute-probe"
    )) {
        crate::kernel::syscall::USER_PC
    } else {
        x86_enter_user as *const () as usize
    }
}
