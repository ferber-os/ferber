//! Retained allocation ownership, independent of the structural Job tree.
#![allow(dead_code)]

use super::heap::Arena;
use crate::kernel::sync::SpinLock;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::fmt;

pub const PAGE_SIZE: usize = 4096;
pub const MAX_JOB_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocError {
    Quota,
    OutOfMemory,
}

struct Usage {
    bytes: usize,
    closed: bool,
}

pub struct JobBudget {
    parent: Option<BudgetRef>,
    limit: usize,
    depth: usize,
    usage: UnsafeCell<Usage>,
    pub(super) arena: UnsafeCell<Arena>,
}
unsafe impl Send for JobBudget {}
unsafe impl Sync for JobBudget {}

static SYSTEM: JobBudget = JobBudget {
    parent: None,
    limit: usize::MAX,
    depth: 0,
    usage: UnsafeCell::new(Usage {
        bytes: 0,
        closed: false,
    }),
    arena: UnsafeCell::new(Arena::new()),
};
static LEDGER: SpinLock<()> = SpinLock::new(());
pub(super) struct LedgerToken(());
pub(super) fn transaction<T>(f: impl FnOnce(&LedgerToken) -> T) -> T {
    let _guard = LEDGER.lock();
    f(&LedgerToken(()))
}

#[derive(Clone)]
pub enum BudgetRef {
    System,
    Job(Arc<JobBudget>),
}
impl fmt::Debug for BudgetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BudgetRef")
            .field("depth", &self.budget().depth)
            .field("limit", &self.budget().limit)
            .finish()
    }
}
impl BudgetRef {
    pub const fn system() -> Self {
        Self::System
    }
    pub(super) fn budget(&self) -> &JobBudget {
        match self {
            Self::System => &SYSTEM,
            Self::Job(budget) => budget,
        }
    }
    pub fn used(&self) -> usize {
        transaction(|_| unsafe { (*self.budget().usage.get()).bytes })
    }
    pub fn limit(&self) -> usize {
        self.budget().limit
    }
    pub fn depth(&self) -> usize {
        self.budget().depth
    }
    pub fn close(&self) {
        transaction(|_| unsafe { (*self.budget().usage.get()).closed = true });
    }
    pub fn is_closed(&self) -> bool {
        transaction(|token| self.open_locked(token).is_err())
    }
    pub fn same_owner(&self, other: &Self) -> bool {
        core::ptr::eq(self.budget(), other.budget())
    }
    pub(super) fn open_locked(&self, _: &LedgerToken) -> Result<(), AllocError> {
        let mut current = Some(self.budget());
        while let Some(budget) = current {
            if unsafe { (*budget.usage.get()).closed } {
                return Err(AllocError::Quota);
            }
            current = budget.parent.as_ref().map(Self::budget);
        }
        Ok(())
    }
    pub(super) fn reserve_locked(
        &self,
        bytes: usize,
        token: &LedgerToken,
    ) -> Result<(), AllocError> {
        self.open_locked(token)?;
        let mut current = Some(self.budget());
        while let Some(budget) = current {
            unsafe { (*budget.usage.get()).bytes }
                .checked_add(bytes)
                .filter(|value| *value <= budget.limit)
                .ok_or(AllocError::Quota)?;
            current = budget.parent.as_ref().map(Self::budget);
        }
        current = Some(self.budget());
        while let Some(budget) = current {
            unsafe {
                (*budget.usage.get()).bytes += bytes;
            }
            current = budget.parent.as_ref().map(Self::budget);
        }
        Ok(())
    }
    pub(super) fn release_locked(&self, bytes: usize, _: &LedgerToken) {
        let mut current = Some(self.budget());
        while let Some(budget) = current {
            let usage = unsafe { &mut *budget.usage.get() };
            usage.bytes = usage
                .bytes
                .checked_sub(bytes)
                .expect("Job charge underflow");
            current = budget.parent.as_ref().map(Self::budget);
        }
    }
    pub fn reserve(&self, bytes: usize) -> Result<(), AllocError> {
        transaction(|token| self.reserve_locked(bytes, token))
    }
    pub fn release(&self, bytes: usize) {
        transaction(|token| self.release_locked(bytes, token));
    }
    pub(super) fn retain_raw(&self) -> *const JobBudget {
        match self {
            Self::System => &SYSTEM,
            Self::Job(budget) => Arc::into_raw(Arc::clone(budget)),
        }
    }
    /// Consume precisely one owner reference saved in an allocation header.
    pub(super) unsafe fn from_raw(raw: *const JobBudget) -> Self {
        if core::ptr::eq(raw, &SYSTEM) {
            Self::System
        } else {
            Self::Job(unsafe { Arc::from_raw(raw) })
        }
    }
    pub(super) unsafe fn clone_raw(raw: *const JobBudget) -> Self {
        if core::ptr::eq(raw, &SYSTEM) {
            Self::System
        } else {
            unsafe {
                Arc::increment_strong_count(raw);
            }
            Self::Job(unsafe { Arc::from_raw(raw) })
        }
    }
}
impl JobBudget {
    pub fn child(parent: &BudgetRef, limit: usize) -> Result<BudgetRef, AllocError> {
        let depth = if matches!(parent, BudgetRef::System) {
            0
        } else {
            parent.depth() + 1
        };
        if parent.is_closed() || depth > MAX_JOB_DEPTH {
            return Err(AllocError::Quota);
        }
        with_budget(parent, || {
            attempt(|| {
                Arc::try_new(Self {
                    parent: Some(parent.clone()),
                    limit,
                    depth,
                    usage: UnsafeCell::new(Usage {
                        bytes: 0,
                        closed: false,
                    }),
                    arena: UnsafeCell::new(Arena::new()),
                })
            })
        })
        .map(BudgetRef::Job)
    }
}
impl Drop for JobBudget {
    fn drop(&mut self) {
        assert_eq!(
            self.usage.get_mut().bytes,
            0,
            "destroying charged Job budget"
        );
        assert!(
            self.arena.get_mut().is_empty(),
            "destroying live Job heap arena"
        );
    }
}

#[derive(Debug)]
pub struct FrameCharge {
    owner: BudgetRef,
    pages: usize,
}
impl FrameCharge {
    pub fn reserve(owner: &BudgetRef, pages: usize) -> Result<Self, AllocError> {
        let bytes = pages.checked_mul(PAGE_SIZE).ok_or(AllocError::Quota)?;
        owner.reserve(bytes)?;
        Ok(Self {
            owner: owner.clone(),
            pages,
        })
    }
    pub fn grow(&mut self, pages: usize) -> Result<(), AllocError> {
        let next = self.pages.checked_add(pages).ok_or(AllocError::Quota)?;
        self.owner
            .reserve(pages.checked_mul(PAGE_SIZE).ok_or(AllocError::Quota)?)?;
        self.pages = next;
        Ok(())
    }
    /// Call only after the corresponding frames have been returned to the bitmap.
    pub fn release(&mut self, pages: usize) {
        self.pages = self
            .pages
            .checked_sub(pages)
            .expect("frame charge underflow");
        self.owner
            .release(pages.checked_mul(PAGE_SIZE).expect("frame charge overflow"));
    }
    pub fn merge(&mut self, mut other: Self) {
        assert!(
            self.owner.same_owner(&other.owner),
            "frame charge owner mismatch"
        );
        self.pages = self
            .pages
            .checked_add(other.pages)
            .expect("frame charge overflow");
        other.pages = 0;
    }
    pub fn pages(&self) -> usize {
        self.pages
    }
    pub fn owner(&self) -> &BudgetRef {
        &self.owner
    }
}
impl Drop for FrameCharge {
    fn drop(&mut self) {
        self.owner.release(self.pages * PAGE_SIZE);
    }
}

struct Context {
    owner: BudgetRef,
    depth: usize,
    failure: *mut Option<AllocError>,
}
impl Context {
    const fn new() -> Self {
        Self {
            owner: BudgetRef::System,
            depth: 0,
            failure: core::ptr::null_mut(),
        }
    }
}
#[cfg(not(test))]
struct CpuContext(UnsafeCell<Context>);
// shortcut: the bootstrap CPU alone allocates with interrupts masked; replace before IRQ/SMP allocation.
#[cfg(not(test))]
unsafe impl Sync for CpuContext {}
#[cfg(not(test))]
static CONTEXT: CpuContext = CpuContext(UnsafeCell::new(Context::new()));
#[cfg(test)]
std::thread_local! { static CONTEXT: core::cell::RefCell<Context> = const { core::cell::RefCell::new(Context::new()) }; }
fn context<T>(f: impl FnOnce(&mut Context) -> T) -> T {
    #[cfg(not(test))]
    {
        f(unsafe { &mut *CONTEXT.0.get() })
    }
    #[cfg(test)]
    {
        CONTEXT.with(|context| f(&mut context.borrow_mut()))
    }
}
pub fn current() -> BudgetRef {
    context(|context| context.owner.clone())
}
pub fn assert_switch_safe() {
    context(|context| {
        assert_eq!(context.depth, 0, "allocation scope crossed context switch");
        assert!(
            context.failure.is_null(),
            "allocation attempt crossed context switch"
        );
    });
}
pub fn set_current(owner: &BudgetRef) {
    assert_switch_safe();
    let previous = context(|context| core::mem::replace(&mut context.owner, owner.clone()));
    drop(previous);
}
pub fn set_system() {
    set_current(&BudgetRef::System);
}
struct Scope {
    previous: Option<BudgetRef>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.previous.take().unwrap();
        let removed = context(|context| {
            context.depth -= 1;
            core::mem::replace(&mut context.owner, previous)
        });
        drop(removed);
    }
}
pub fn with_budget<T>(owner: &BudgetRef, f: impl FnOnce() -> T) -> T {
    let previous = context(|context| {
        context.depth = context
            .depth
            .checked_add(1)
            .expect("allocation scope depth overflow");
        core::mem::replace(&mut context.owner, owner.clone())
    });
    let _scope = Scope {
        previous: Some(previous),
    };
    f()
}
struct Attempt {
    previous: *mut Option<AllocError>,
}
impl Drop for Attempt {
    fn drop(&mut self) {
        context(|context| context.failure = self.previous);
    }
}
pub fn attempt<T, E>(f: impl FnOnce() -> Result<T, E>) -> Result<T, AllocError> {
    let mut error = None;
    let previous = context(|context| core::mem::replace(&mut context.failure, &mut error));
    let _attempt = Attempt { previous };
    f().map_err(|_| error.unwrap_or(AllocError::OutOfMemory))
}
pub(super) fn failed(error: AllocError) {
    context(|context| {
        if !context.failure.is_null() {
            unsafe {
                *context.failure = Some(error);
            }
        }
    });
}
static BOOTSTRAP_ADOPTED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
pub fn adopt_bootstrap(pages: usize) {
    assert!(
        !BOOTSTRAP_ADOPTED.swap(true, core::sync::atomic::Ordering::AcqRel),
        "bootstrap charges adopted twice"
    );
    BudgetRef::System
        .reserve(
            pages
                .checked_mul(PAGE_SIZE)
                .expect("bootstrap charge overflow"),
        )
        .expect("System bootstrap accounting");
}
