//! Thread execution container.

#![allow(dead_code)]

extern crate alloc;
use alloc::borrow::Cow;
use alloc::sync::Weak;
use core::cell::UnsafeCell;
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
    name: Cow<'static, str>,
    process: Option<Weak<SpinLock<Process>>>,
    state: ThreadState,
    context: UnsafeCell<CpuContext>,
    user_entry: usize,
    kernel_stack: Vmo,
    user_stack: Option<Vmo>,
    user_stack_base: Option<usize>,
    user_initial_sp: Option<usize>,
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
        name: impl Into<Cow<'static, str>>,
        entry: usize,
        kstack: Vmo,
        kstack_top: usize,
        process: Option<Weak<SpinLock<Process>>>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            process,
            state: ThreadState::Ready,
            context: UnsafeCell::new(CpuContext::new(entry, kstack_top)),
            user_entry: entry,
            kernel_stack: kstack,
            user_stack: None,
            user_stack_base: None,
            user_initial_sp: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
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

    pub fn user_entry(&self) -> usize {
        self.user_entry
    }

    pub fn kernel_stack_top(&self) -> usize {
        let physical = self.kernel_stack.frames().get(0).expect("kernel stack frame");
        crate::kernel::memory::phys_map::phys_to_virt(physical)
            .expect("kernel stack alias")
            + self.kernel_stack.size()
    }

    pub(crate) fn initialize_context(&mut self, entry: usize) {
        *self.context.get_mut() = CpuContext::new(entry, self.kernel_stack_top());
    }

    /// # Safety
    /// The sole active CPU must retain this thread, release its lock before
    /// switching, and ensure no other execution accesses the saved context.
    pub(crate) unsafe fn context_ptr(&self) -> *mut CpuContext {
        self.context.get()
    }

    pub fn user_stack(&self) -> Option<&Vmo> {
        self.user_stack.as_ref()
    }

    pub fn set_user_stack(&mut self, ustack: Vmo) {
        self.user_stack = Some(ustack);
        self.user_initial_sp = None;
    }

    pub fn set_user_stack_mapping(&mut self, base: usize) {
        self.user_stack_base = Some(base);
        self.user_initial_sp = None;
    }

    pub(crate) fn set_user_initial_sp(&mut self, sp: usize) -> Result<(), ()> {
        let (base, size) = self.user_stack_mapping().ok_or(())?;
        let top = base.checked_add(size).ok_or(())?;
        if sp & 15 != 0 || sp <= base || sp > top {
            return Err(());
        }
        self.user_initial_sp = Some(sp);
        Ok(())
    }

    pub fn user_initial_sp(&self) -> Option<usize> {
        let (base, size) = self.user_stack_mapping()?;
        self.user_initial_sp.or_else(|| base.checked_add(size))
    }

    pub fn user_stack_mapping(&self) -> Option<(usize, usize)> {
        Some((self.user_stack_base?, self.user_stack.as_ref()?.size()))
    }
}
