//! Resident integer-only probes and the initial privilege transition.

use core::arch::global_asm;

global_asm!(
    include_str!("user.S"),
    sys_yield = const crate::kernel::syscall::SYS_YIELD,
    sys_exit = const crate::kernel::syscall::SYS_EXIT,
    sys_close = const crate::kernel::syscall::SYS_HANDLE_CLOSE,
    sys_duplicate = const crate::kernel::syscall::SYS_HANDLE_DUPLICATE,
    sys_attenuate = const crate::kernel::syscall::SYS_CAP_ATTENUATE,
    sys_revoke = const crate::kernel::syscall::SYS_CAP_REVOKE,
    sys_inspect = const crate::kernel::syscall::SYS_CAP_INSPECT,
    user_stack = const crate::kernel::syscall::USER_STACK,
    stack_size = const crate::kernel::syscall::STACK_SIZE,
    write_probe = const cfg!(feature = "user-write-probe") as usize,
    user_pc = const crate::kernel::syscall::USER_PC,
);

extern "C" {
    fn riscv_drop_to_user(user_pc: usize, user_sp: usize, kernel_sp: usize) -> !;
    static riscv_user_probe_start: u8;
    static riscv_user_probe_end: u8;
    static riscv_isolation_probe_start: u8;
    static riscv_isolation_probe_end: u8;
    static riscv_isolation_fault: u8;
}

/// Drop to U-mode. Does not return.
///
/// # Safety
/// The active root must map the entry and user stack with user permissions and
/// this code, the current kernel state, and the contiguous kernel stack as
/// supervisor memory. All mappings and their VMO backing must outlive execution.
// named after edesur, the local power company.
// they also drop everything without warning and never return.
// 2026-10-14 22:56 AST: power cut mid-build. this comment is the only thing
// that survived. edesur giveth, edesur taketh. edesur mostly taketh.
pub unsafe fn drop_to_user(user_pc: usize, user_sp: usize, kernel_sp: usize) -> ! {
    assert_eq!(kernel_sp & 15, 0);
    assert_eq!(user_sp & 15, 0);
    riscv_drop_to_user(user_pc, user_sp, kernel_sp)
}

pub fn user_probe() -> &'static [u8] {
    unsafe {
        let start = &riscv_user_probe_start as *const u8;
        let len = (&riscv_user_probe_end as *const u8 as usize) - start as usize;
        core::slice::from_raw_parts(start, len)
    }
}

pub fn isolation_probe() -> &'static [u8] {
    unsafe {
        let start = &riscv_isolation_probe_start as *const u8;
        let len = (&riscv_isolation_probe_end as *const u8 as usize) - start as usize;
        core::slice::from_raw_parts(start, len)
    }
}

pub fn isolation_fault_offset() -> usize {
    unsafe {
        (&riscv_isolation_fault as *const u8 as usize)
            - (&riscv_isolation_probe_start as *const u8 as usize)
    }
}

pub fn isolation_address() -> usize {
    if cfg!(any(
        feature = "user-write-probe",
        feature = "user-execute-probe"
    )) {
        crate::kernel::syscall::USER_PC
    } else {
        0x80200000
    }
}
