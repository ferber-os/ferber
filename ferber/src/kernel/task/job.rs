//! Job container.
//!
//! Enforces hierarchical Job tree with zero orphans: killing a parent Job
//! recursively kills all child jobs, processes, and threads.

#![allow(dead_code)]

extern crate alloc;
use super::process::Process;
use super::TaskError;
use crate::arch::ArchAddressSpace;
use crate::kernel::capabilities::KernelObject;
use crate::kernel::memory::accounting::{self, AllocError, BudgetRef, JobBudget};
use crate::kernel::sync::SpinLock;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Active,
    Killed,
}

pub struct Job {
    id: u64,
    name: &'static str,
    parent: Option<Weak<SpinLock<Job>>>,
    children: Vec<Arc<SpinLock<Job>>>,
    processes: Vec<Arc<SpinLock<Process>>>,
    budget: BudgetRef,
    state: JobState,
    self_weak: Option<Weak<SpinLock<Job>>>,
}

unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl KernelObject for Job {
    fn type_name(&self) -> &'static str {
        "Job"
    }
}

impl Job {
    pub fn create_root(memory_limit: usize) -> Arc<SpinLock<Job>> {
        let budget = JobBudget::child(&BudgetRef::system(), memory_limit).expect("root Job budget");
        let job = Arc::new(SpinLock::new(Self {
            id: 1,
            name: "root",
            parent: None,
            children: Vec::new(),
            processes: Vec::new(),
            budget,
            state: JobState::Active,
            self_weak: None,
        }));
        job.lock().self_weak = Some(Arc::downgrade(&job));
        job
    }

    pub fn create_child_job(
        &mut self,
        name: &'static str,
        memory_limit: usize,
    ) -> Result<Arc<SpinLock<Job>>, TaskError> {
        if self.state == JobState::Killed {
            return Err(TaskError::JobKilled);
        }
        let child_id = NEXT_JOB_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| TaskError::QuotaExceeded)?;
        let budget = JobBudget::child(&self.budget, memory_limit).map_err(alloc_error)?;
        accounting::with_budget(&self.budget, || {
            accounting::attempt(|| self.children.try_reserve(1)).map_err(alloc_error)?;
            let child = accounting::attempt(|| {
                Arc::try_new(SpinLock::new(Job {
                    id: child_id,
                    name,
                    parent: self.self_weak.clone(),
                    children: Vec::new(),
                    processes: Vec::new(),
                    budget,
                    state: JobState::Active,
                    self_weak: None,
                }))
            })
            .map_err(alloc_error)?;
            child.lock().self_weak = Some(Arc::downgrade(&child));
            self.children.push(Arc::clone(&child));
            Ok(child)
        })
    }

    pub fn create_process(
        &mut self,
        name: &'static str,
    ) -> Result<Arc<SpinLock<Process>>, TaskError> {
        if self.state == JobState::Killed {
            return Err(TaskError::JobKilled);
        }
        let permit = super::process::ProcessPermit::reserve()?;
        let budget = self.budget.clone();
        accounting::with_budget(&budget, || {
            accounting::attempt(|| self.processes.try_reserve(1)).map_err(alloc_error)?;
            let address_space = ArchAddressSpace::new_user().map_err(|error| {
                if matches!(
                    error,
                    crate::kernel::memory::paging::PageError::TableBudget
                        | crate::kernel::memory::paging::PageError::JobQuota
                ) {
                    TaskError::QuotaExceeded
                } else {
                    TaskError::OutOfMemory
                }
            })?;
            let proc_id = super::process::next_process_id()?;
            let process = accounting::attempt(|| {
                Arc::try_new(SpinLock::new(Process::new(
                    proc_id,
                    name,
                    address_space,
                    self.self_weak.clone(),
                )))
            })
            .map_err(alloc_error)?;
            process.lock().set_self_weak(Arc::downgrade(&process));
            process.lock().retain_process_permit(permit);
            self.processes.push(Arc::clone(&process));
            Ok(process)
        })
    }

    pub fn kill(&mut self) {
        if self.state == JobState::Killed {
            return;
        }
        self.budget.close();
        self.state = JobState::Killed;
        for child in &self.children {
            child.lock().kill();
        }
        for process in &self.processes {
            process.lock().kill();
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn state(&self) -> JobState {
        self.state
    }

    pub fn children_count(&self) -> usize {
        self.children.len()
    }

    pub fn process_count(&self) -> usize {
        self.processes.len()
    }

    pub fn memory_limit(&self) -> usize {
        self.budget.limit()
    }

    pub fn memory_used(&self) -> usize {
        self.budget.used()
    }

    pub fn budget(&self) -> &BudgetRef {
        &self.budget
    }

    pub fn parent(&self) -> Option<Weak<SpinLock<Job>>> {
        self.parent.clone()
    }

    pub fn children(&self) -> &[Arc<SpinLock<Job>>] {
        &self.children
    }

    pub fn processes(&self) -> &[Arc<SpinLock<Process>>] {
        &self.processes
    }

    pub(crate) fn detach_child_job(&mut self, id: u64) -> Option<Arc<SpinLock<Job>>> {
        let index = self
            .children
            .iter()
            .position(|child| child.lock().id() == id)?;
        Some(self.children.remove(index))
    }

    pub fn detach_process(&mut self, id: u64) -> Option<Arc<SpinLock<Process>>> {
        let index = self
            .processes
            .iter()
            .position(|process| process.lock().id() == id)?;
        Some(self.processes.remove(index))
    }
}

fn alloc_error(error: AllocError) -> TaskError {
    match error {
        AllocError::Quota => TaskError::QuotaExceeded,
        AllocError::OutOfMemory => TaskError::OutOfMemory,
    }
}
