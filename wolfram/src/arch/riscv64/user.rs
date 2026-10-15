//! Resident integer-only probes and the initial privilege transition.

use core::arch::global_asm;

global_asm!(include_str!("user.S"));

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
        assert!(len <= 4096);
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
