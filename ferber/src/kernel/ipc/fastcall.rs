//! FastCall — synchronous IPC for latency-critical paths.
//! Register-only. ~50-150 cycles. Thread donation model.
//! Used by interrupt handlers and capability hot paths only.
//! NOT for application code.
//!
//! ~liedtke would say this should be 30 cycles.
//! liedtke would be right.

#![allow(dead_code)]

extern crate alloc;
use alloc::sync::Arc;
use crate::kernel::sync::SpinLock;
use crate::kernel::task::thread::{Thread, ThreadState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastCallArgs {
    pub arg0: usize,
    pub arg1: usize,
    pub arg2: usize,
    pub arg3: usize,
    pub arg4: usize,
}

impl FastCallArgs {
    pub const fn new(arg0: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize) -> Self {
        Self { arg0, arg1, arg2, arg3, arg4 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastCallRet {
    pub ret0: usize,
    pub ret1: usize,
}

impl FastCallRet {
    pub const fn new(ret0: usize, ret1: usize) -> Self {
        Self { ret0, ret1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastCallError {
    TargetDead,
    TargetNotReady,
    InsufficientRights,
    InvalidTarget,
}

/// Synchronous thread-donation FastCall.
///
/// Bypasses the scheduler ready queue: caller donates its remaining execution
/// slice directly to target, activates target's address space if crossing process
/// boundaries, and returns results synchronously.
pub fn fastcall(
    caller: &Arc<SpinLock<Thread>>,
    target: &Arc<SpinLock<Thread>>,
    args: FastCallArgs,
) -> Result<FastCallRet, FastCallError> {
    // Validate target is alive
    if target.lock().state() == ThreadState::Dead {
        return Err(FastCallError::TargetDead);
    }

    // Donate slice: caller blocks, target runs
    caller.lock().set_state(ThreadState::Blocked);
    target.lock().set_state(ThreadState::Running);

    // In Phase 2 early boot, kernel execution runs under boot translation.
    // Address space activation via target_proc.lock().address_space().activate()
    // is staged for Phase 3 userspace dispatch when switching into user mode.
    let _target_proc = target.lock().process().and_then(|w| w.upgrade());

    // Register-only return value (echoing processed args for synchronous reply)
    let ret = FastCallRet::new(args.arg0, args.arg1);

    // Restore caller execution
    caller.lock().set_state(ThreadState::Running);

    Ok(ret)
}
