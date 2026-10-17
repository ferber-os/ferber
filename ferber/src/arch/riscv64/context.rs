//! RISC-V 64 thread context switching.

#![allow(dead_code)]

use core::arch::global_asm;

global_asm!(include_str!("context.S"));

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuContext {
    pub ra: usize,
    pub sp: usize,
    pub s0: usize,
    pub s1: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
}

const _: () = assert!(core::mem::size_of::<CpuContext>() == 14 * 8);

impl CpuContext {
    pub const fn empty() -> Self {
        Self { ra: 0, sp: 0, s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0, s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0 }
    }

    pub fn new(entry: usize, kstack_top: usize) -> Self {
        Self {
            ra: entry,
            sp: kstack_top,
            s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0, s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0,
        }
    }
}

extern "C" {
    pub fn switch_context(prev: *mut CpuContext, next: *const CpuContext);
}
