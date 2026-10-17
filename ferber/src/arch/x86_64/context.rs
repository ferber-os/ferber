//! x86-64 thread context switching.

#![allow(dead_code)]

use core::arch::global_asm;

global_asm!(include_str!("context.S"));

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuContext {
    pub rsp: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
}

const _: () = assert!(core::mem::size_of::<CpuContext>() == 7 * 8);

impl CpuContext {
    pub const fn empty() -> Self {
        Self { rsp: 0, rbp: 0, rbx: 0, r12: 0, r13: 0, r14: 0, r15: 0 }
    }

    /// Creates a new context whose kernel stack has been initialized such that
    /// when `switch_context` restores `rsp` and executes `ret`, it jumps to `entry`.
    pub fn new(entry: usize, kstack_top: usize) -> Self {
        // kstack_top must be 16-byte aligned. We reserve 8 bytes to push `entry` as return address.
        let aligned_top = kstack_top & !0xf;
        let sp = aligned_top - 8;
        unsafe {
            (sp as *mut usize).write(entry);
        }
        Self {
            rsp: sp,
            rbp: 0,
            rbx: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }
    }
}

extern "C" {
    pub fn switch_context(prev: *mut CpuContext, next: *const CpuContext);
}
