//! Ferber Scheduler
//! Jobs -> Processes -> Threads. No fork. Explicit spawn. Born with nothing.

#![allow(dead_code)]

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use crate::kernel::sync::SpinLock;
use crate::kernel::task::thread::{Thread, ThreadState};

static READY_QUEUE: SpinLock<VecDeque<Arc<SpinLock<Thread>>>> = SpinLock::new(VecDeque::new());

pub fn spawn(thread: Arc<SpinLock<Thread>>) {
    READY_QUEUE.lock().push_back(thread);
}

pub fn ready_count() -> usize {
    READY_QUEUE.lock().len()
}

pub fn sched_yield() {
    // Pops next ready thread and executes switch_context (staged for M3 full dispatch loop)
    let next_thread = {
        let mut queue = READY_QUEUE.lock();
        match queue.pop_front() {
            Some(thread) => thread,
            None => return,
        }
    };
    next_thread.lock().set_state(ThreadState::Running);
}

pub fn init() {}
