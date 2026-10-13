//! Process container.
//!
//! Enforces zero ambient authority: processes are born with an empty HandleTable.

#![allow(dead_code)]

extern crate alloc;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use crate::arch::ArchAddressSpace;
use crate::kernel::capabilities::{KernelObject, HandleTable};
use crate::kernel::memory::vmo::Vmo;
use crate::kernel::sync::SpinLock;
use super::thread::{Thread, ThreadState};
use super::job::Job;
use super::TaskError;

static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_process_id() -> u64 {
    NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Created,
    Running,
    Suspended,
    Dead,
}

pub struct Process {
    id: u64,
    name: &'static str,
    job: Option<Weak<SpinLock<Job>>>,
    state: ProcessState,
    address_space: ArchAddressSpace,
    handle_table: SpinLock<HandleTable>,
    threads: Vec<Arc<SpinLock<Thread>>>,
    self_weak: Option<Weak<SpinLock<Process>>>,
}

unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl KernelObject for Process {
    fn type_name(&self) -> &'static str {
        "Process"
    }
}

impl Process {
    pub fn new(
        id: u64,
        name: &'static str,
        address_space: ArchAddressSpace,
        job: Option<Weak<SpinLock<Job>>>,
    ) -> Self {
        Self {
            id,
            name,
            job,
            state: ProcessState::Created,
            address_space,
            handle_table: SpinLock::new(HandleTable::new()),
            threads: Vec::new(),
            self_weak: None,
        }
    }

    pub fn create_thread(
        &mut self,
        entry: usize,
        kstack_size: usize,
    ) -> Result<Arc<SpinLock<Thread>>, TaskError> {
        let name = alloc::boxed::Box::leak(
            alloc::format!("{}/thread_{}", self.name, self.threads.len()).into_boxed_str(),
        );
        self.create_named_thread(name, entry, kstack_size)
    }

    pub fn create_named_thread(
        &mut self,
        name: &'static str,
        entry: usize,
        kstack_size: usize,
    ) -> Result<Arc<SpinLock<Thread>>, TaskError> {
        if self.state == ProcessState::Dead {
            return Err(TaskError::ProcessDead);
        }
        let kstack = Vmo::create_contiguous(kstack_size)
            .or_else(|_| Vmo::create(kstack_size))
            .map_err(|_| TaskError::OutOfMemory)?;
        let kstack_base = kstack.frames().get(0).ok_or(TaskError::OutOfMemory)?;
        let kstack_top = kstack_base + kstack_size;
        let thread_id = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
        let thread = Arc::new(SpinLock::new(Thread::new(
            thread_id,
            name,
            entry,
            kstack,
            kstack_top,
            self.self_weak.clone(),
        )));
        self.threads.push(Arc::clone(&thread));
        Ok(thread)
    }

    pub fn kill(&mut self) {
        if self.state == ProcessState::Dead {
            return;
        }
        self.state = ProcessState::Dead;
        for thread in &self.threads {
            thread.lock().set_state(ThreadState::Dead);
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn state(&self) -> ProcessState {
        self.state
    }

    pub fn set_state(&mut self, state: ProcessState) {
        self.state = state;
    }

    pub fn handle_table(&self) -> &SpinLock<HandleTable> {
        &self.handle_table
    }

    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }

    pub fn threads(&self) -> &[Arc<SpinLock<Thread>>] {
        &self.threads
    }

    pub fn address_space(&self) -> &ArchAddressSpace {
        &self.address_space
    }

    pub fn address_space_mut(&mut self) -> &mut ArchAddressSpace {
        &mut self.address_space
    }

    pub fn job(&self) -> Option<Weak<SpinLock<Job>>> {
        self.job.clone()
    }

    pub fn set_job(&mut self, job: Weak<SpinLock<Job>>) {
        self.job = Some(job);
    }

    pub(crate) fn set_self_weak(&mut self, weak: Weak<SpinLock<Process>>) {
        self.self_weak = Some(weak);
    }
}
