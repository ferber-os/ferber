//! Thread execution container.

#![allow(dead_code)]

extern crate alloc;
use alloc::sync::Weak;
use crate::arch::CpuContext;
use crate::kernel::capabilities::KernelObject;
use crate::kernel::memory::vmo::Vmo;
use crate::kernel::sync::SpinLock;
use super::process::Process;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked,
    Dead,
}

pub struct Thread {
    id: u64,
    name: &'static str,
    process: Option<Weak<SpinLock<Process>>>,
    state: ThreadState,
    pub context: CpuContext,
    kernel_stack: Vmo,
    user_stack: Option<Vmo>,
}

unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}

impl KernelObject for Thread {
    fn type_name(&self) -> &'static str {
        "Thread"
    }
}

impl Thread {
    pub fn new(
        id: u64,
        name: &'static str,
        entry: usize,
        kstack: Vmo,
        kstack_top: usize,
        process: Option<Weak<SpinLock<Process>>>,
    ) -> Self {
        Self {
            id,
            name,
            process,
            state: ThreadState::Ready,
            context: CpuContext::new(entry, kstack_top),
            kernel_stack: kstack,
            user_stack: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn state(&self) -> ThreadState {
        self.state
    }

    pub fn set_state(&mut self, state: ThreadState) {
        self.state = state;
    }

    pub fn process(&self) -> Option<Weak<SpinLock<Process>>> {
        self.process.clone()
    }

    pub fn set_process(&mut self, process: Weak<SpinLock<Process>>) {
        self.process = Some(process);
    }

    pub fn kernel_stack(&self) -> &Vmo {
        &self.kernel_stack
    }

    pub fn user_stack(&self) -> Option<&Vmo> {
        self.user_stack.as_ref()
    }

    pub fn set_user_stack(&mut self, ustack: Vmo) {
        self.user_stack = Some(ustack);
    }
}
