//! Single-CPU syscall entry. Interrupts remain masked until timer scheduling exists.

use core::arch::{asm, global_asm};

global_asm!(include_str!("syscall.S"));

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
    fn x86_enter_user(pc: usize, sp: usize) -> !;
    static x86_user_probe_start: u8;
    static x86_user_probe_end: u8;
    static x86_isolation_start: u8;
    static x86_isolation_end: u8;
    static x86_isolation_fault: u8;
}

#[no_mangle]
extern "C" fn x86_syscall_handler(frame: &mut SyscallFrame) {
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
    frame.rflags = (frame.rflags & 0x8d5) | 2;
}

/// Enter ring 3. Does not return.
///
/// # Safety
/// The active root must retain the kernel and executable user mappings; the
/// supplied user and kernel stacks must remain allocated and correctly aligned.
pub unsafe fn drop_to_user(user_pc: usize, user_sp: usize, kernel_sp: usize) -> ! {
    assert_eq!(kernel_sp & 15, 0);
    assert_eq!(user_sp & 15, 0);
    super::trap::set_kernel_stack(kernel_sp);
    CPU.kernel_sp = kernel_sp;
    CPU.user_sp = 0;
    asm!("xor eax, eax", "mov fs, ax", "mov gs, ax", out("eax") _, options(nostack));
    write_msr(0xc000_0100, 0);
    write_msr(0xc000_0101, (&raw mut CPU) as u64);
    write_msr(0xc000_0102, 0);
    x86_enter_user(user_pc, user_sp)
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
    x86_enter_user as *const () as usize
}
