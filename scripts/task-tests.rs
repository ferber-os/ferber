//! Production task syscalls, containers, capabilities, VMOs and copy validation.
//! Host page tables and scheduler replace hardware; heap metadata uses host storage.
#![feature(allocator_ext)]
#![allow(dead_code, unused_imports)]
extern crate alloc;
#[path = "accounting-host.rs"]
mod accounting_host;
pub use accounting_host::accounting;
#[path = "../ferber/src/kernel/capabilities/mod.rs"]
pub mod capabilities;
#[path = "../ferber/src/kernel/ipc/channel.rs"]
pub mod channel;
#[path = "../ferber/src/kernel/memory/paging.rs"]
pub mod paging;
#[path = "../shared/startup.rs"]
pub mod startup;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;
#[path = "../ferber/src/kernel/syscall_task.rs"]
pub mod syscall_task;
#[path = "../ferber/src/kernel/task/mod.rs"]
pub mod task;
#[path = "../ferber/src/kernel/memory/user_mem.rs"]
pub mod user_mem;
#[path = "../ferber/src/kernel/memory/vmo.rs"]
pub mod vmo;
use alloc::sync::Arc;
use kernel::{ipc, memory, syscall};
use paging::{AddressSpace, MapFlags, PageError};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{LazyLock, Mutex},
};
use sync::SpinLock;
use task::{Job, Process};
static SERIAL: Mutex<()> = Mutex::new(());
thread_local! { static FAIL: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL
            .try_with(|left| match left.get() {
                Some(0) => {
                    left.set(None);
                    true
                }
                Some(n) => {
                    left.set(Some(n - 1));
                    false
                }
                None => false,
            })
            .unwrap_or(false)
        {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(p, layout);
        }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
#[macro_export]
macro_rules! kprintln { ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}; }
mod kernel {
    pub use crate::{capabilities, scheduler, startup, sync, task};
    pub mod ipc {
        pub use crate::channel;
    }
    pub mod memory {
        pub use crate::{accounting, bitmap, paging, phys_map, user_mem, vmo};
        pub use vmo::Vmo;
    }
    pub mod syscall {
        pub const USER_STACK: usize = 0x20_0000_0000;
        pub const STACK_SIZE: usize = 16384;
        pub fn allocation_errno(error: crate::accounting::AllocError) -> isize {
            match error {
                crate::accounting::AllocError::Quota => -28,
                _ => -12,
            }
        }
        pub fn cap_errno(error: crate::capabilities::CapError) -> isize {
            use crate::capabilities::CapError::*;
            match error {
                InvalidHandle => -9,
                InsufficientRights => -13,
                TableFull => -24,
                Revoked => -116,
                OutOfMemory => -12,
                NodeCapacity | JobQuota => -28,
                InvalidRights => -22,
            }
        }
    }
}
pub mod phys_map {
    pub fn phys_to_virt(address: usize) -> Option<usize> {
        Some(address)
    }
    pub fn virt_to_phys(address: usize) -> Option<usize> {
        Some(address)
    }
    pub fn mmio_to_virt(_: usize) -> Option<usize> {
        None
    }
}
pub mod bitmap {
    use super::*;
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicIsize, Ordering},
    };
    pub static FAIL_FRAME: AtomicIsize = AtomicIsize::new(-1);
    static FRAMES: LazyLock<Mutex<BTreeMap<usize, (usize, Layout)>>> =
        LazyLock::new(|| Mutex::new(BTreeMap::new()));
    pub fn count() -> usize {
        FRAMES.lock().unwrap().len()
    }
    pub struct Stats {
        pub free_pages: usize,
    }
    pub fn stats() -> Stats {
        Stats {
            free_pages: 1_000_000 - count(),
        }
    }
    pub fn allocate_frame() -> Option<usize> {
        allocate_frames(1)
    }
    pub fn allocate_frames(count: usize) -> Option<usize> {
        if FAIL_FRAME
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                if n < 0 {
                    Some(n)
                } else {
                    n.checked_sub(1)
                }
            })
            .is_err()
        {
            return None;
        }
        let layout = Layout::from_size_align(count * 4096, 4096).ok()?;
        let pointer = unsafe { System.alloc(layout) };
        if pointer.is_null() {
            return None;
        }
        let previous = FAIL.with(|f| f.replace(None));
        let mut frames = FRAMES.lock().unwrap();
        for i in 0..count {
            frames.insert(pointer as usize + i * 4096, (pointer as usize, layout));
        }
        FAIL.with(|f| f.set(previous));
        Some(pointer as usize)
    }
    pub fn free_frame(frame: usize) -> Result<(), ()> {
        let mut frames = FRAMES.lock().unwrap();
        let (base, layout) = frames.remove(&frame).ok_or(())?;
        if !frames.values().any(|(other, _)| *other == base) {
            unsafe {
                System.dealloc(base as *mut u8, layout);
            }
        }
        Ok(())
    }
}
pub mod arch {
    use super::*;
    #[derive(Default)]
    pub struct CpuContext;
    impl CpuContext {
        pub fn new(_: usize, _: usize) -> Self {
            Self
        }
    }
    pub struct ArchAddressSpace {
        entries: Vec<(usize, usize, MapFlags)>,
        owner: accounting::BudgetRef,
    }
    impl ArchAddressSpace {
        pub fn new() -> Result<Self, PageError> {
            Self::new_user()
        }
        pub fn new_user() -> Result<Self, PageError> {
            Ok(Self {
                entries: Vec::new(),
                owner: accounting::current(),
            })
        }
        pub fn budget(&self) -> &accounting::BudgetRef {
            &self.owner
        }
    }
    impl AddressSpace for ArchAddressSpace {
        unsafe fn map_page(
            &mut self,
            va: usize,
            pa: usize,
            flags: MapFlags,
        ) -> Result<(), PageError> {
            self.check_free_range(va, 1)?;
            self.entries
                .try_reserve(1)
                .map_err(|_| PageError::OutOfMemory)?;
            self.entries.push((va, pa, flags));
            Ok(())
        }
        unsafe fn unmap_page(&mut self, va: usize) -> Result<usize, PageError> {
            let i = self
                .entries
                .iter()
                .position(|e| e.0 == va)
                .ok_or(PageError::NotMapped)?;
            Ok(self.entries.remove(i).1)
        }
        unsafe fn protect_page(&mut self, va: usize, flags: MapFlags) -> Result<(), PageError> {
            let e = self
                .entries
                .iter_mut()
                .find(|e| e.0 == va)
                .ok_or(PageError::NotMapped)?;
            e.2 = flags;
            Ok(())
        }
        fn query(&self, va: usize) -> Option<(usize, MapFlags)> {
            self.entries
                .iter()
                .find(|e| e.0 == va & !4095)
                .map(|e| (e.1 + (va & 4095), e.2))
        }
        fn check_free_range(&self, va: usize, count: usize) -> Result<(), PageError> {
            if self
                .entries
                .iter()
                .any(|e| va <= e.0 && e.0 < va + count * 4096)
            {
                Err(PageError::AlreadyMapped)
            } else {
                Ok(())
            }
        }
        unsafe fn activate(&self) {}
        unsafe fn flush_tlb_page(&self, _: usize) {}
        unsafe fn flush_tlb_all(&self) {}
        unsafe fn map_pages(
            &mut self,
            va: usize,
            frames: &vmo::VmoFrames,
            first: usize,
            count: usize,
            flags: MapFlags,
        ) -> Result<(), PageError> {
            self.check_free_range(va, count)?;
            self.entries
                .try_reserve(count)
                .map_err(|_| PageError::OutOfMemory)?;
            for i in 0..count {
                self.entries
                    .push((va + i * 4096, frames.get(first + i).unwrap(), flags));
            }
            Ok(())
        }
        unsafe fn unmap_pages_exact(
            &mut self,
            va: usize,
            frames: &vmo::VmoFrames,
            first: usize,
            count: usize,
            flags: MapFlags,
        ) -> Result<(), PageError> {
            for i in 0..count {
                if self.query(va + i * 4096) != Some((frames.get(first + i).unwrap(), flags)) {
                    return Err(PageError::NotMapped);
                }
            }
            for i in 0..count {
                unsafe {
                    self.unmap_page(va + i * 4096)?;
                }
            }
            Ok(())
        }
    }
}
pub mod scheduler {
    use super::*;
    type Task = (Arc<SpinLock<Process>>, Arc<SpinLock<task::Thread>>);
    pub static FAIL_START: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static TASKS: Mutex<Vec<Task>> = Mutex::new(Vec::new());
    pub fn admit(
        process: Arc<SpinLock<Process>>,
        thread: Arc<SpinLock<task::Thread>>,
    ) -> Result<(), task::TaskError> {
        if FAIL_START.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(task::TaskError::OutOfMemory);
        }
        let mut tasks = TASKS.lock().unwrap();
        tasks
            .try_reserve(1)
            .map_err(|_| task::TaskError::OutOfMemory)?;
        tasks.push((process, thread));
        Ok(())
    }
    pub fn count() -> usize {
        TASKS.lock().unwrap().len()
    }
    pub fn process() -> Arc<SpinLock<Process>> {
        Arc::clone(&TASKS.lock().unwrap()[0].0)
    }
    pub fn reap() {
        for (process, _) in TASKS.lock().unwrap().drain(..) {
            let (job, id, handles, mappings) = {
                let mut p = process.lock();
                p.kill();
                (p.job(), p.id(), p.take_handles(), p.take_mappings())
            };
            drop(mappings);
            drop(handles);
            if let Some(job) = job.and_then(|j| j.upgrade()) {
                drop(job.lock().detach_process(id));
            }
        }
    }
}
struct Fixture {
    root: Arc<SpinLock<Job>>,
    caller: Arc<SpinLock<Process>>,
    job: usize,
}
impl Fixture {
    fn new() -> Self {
        assert_eq!(task::process::live_processes(), 0);
        let root = Job::create_root(64 * 1024 * 1024);
        let caller = root.lock().create_process("caller").unwrap();
        let cap = syscall_task::job_capability(&root).unwrap();
        let job = caller.lock().handle_table().lock().insert(cap).unwrap() as usize;
        Self { root, caller, job }
    }
    fn call(&self, nr: usize, args: [usize; 6]) -> isize {
        syscall_task::dispatch(&self.caller, nr, args)
    }
    fn create(&self) -> usize {
        let h = self.call(17, [self.job, 0, 0, 0, 0, 0]);
        assert!(h >= 0, "create {h}");
        h as usize
    }
    fn close(&self, handle: usize) {
        let node = self
            .caller
            .lock()
            .handle_table()
            .lock()
            .remove(handle as u32)
            .unwrap();
        drop(node);
    }
    fn cap(
        &self,
        object: Arc<dyn capabilities::KernelObject>,
        kind: capabilities::ObjectType,
        rights: capabilities::Rights,
    ) -> usize {
        let node = capabilities::CAP_NODE_ALLOCATOR
            .alloc_root(object, kind, rights)
            .unwrap();
        self.caller
            .lock()
            .handle_table()
            .lock()
            .insert(node)
            .unwrap() as usize
    }
    fn prepare(&self, control: usize) {
        self.prepare_bootstrap(control, usize::MAX);
    }
    fn prepare_bootstrap(&self, control: usize, bootstrap: usize) {
        use capabilities::{ObjectType, Rights};
        let memory = Arc::new(vmo::Vmo::create(4096).unwrap());
        memory.seal().unwrap();
        let handle = self.cap(
            memory,
            ObjectType::Vmo,
            Rights::READ | Rights::MAP | Rights::EXECUTE,
        );
        assert_eq!(self.map(control, handle, [0x10000, 0, 4096, 5, 0, 0]), 0);
        self.close(handle);
        assert_eq!(self.call(20, [control, 0x10000, bootstrap, 0, 0, 0]), 0);
    }
    fn map(&self, control: usize, handle: usize, descriptor: [u64; 6]) -> isize {
        let mut buffer = vmo::Vmo::create(4096).unwrap();
        let mut bytes = [0; 48];
        for (to, value) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(descriptor) {
            *to = value.to_le_bytes();
        }
        buffer.write(0, &bytes).unwrap();
        {
            let mut caller = self.caller.lock();
            unsafe {
                caller
                    .address_space_mut()
                    .map_vmo(0x1000, buffer.frames(), MapFlags::user_ro())
                    .unwrap();
            }
        }
        let result = self.call(18, [control, handle, 0x1000, 0, 0, 0]);
        {
            let mut caller = self.caller.lock();
            unsafe {
                caller.address_space_mut().unmap_vmo(0x1000, 1).unwrap();
            }
        }
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        scheduler::reap();
        let handles = self.caller.lock().take_handles();
        drop(handles);
        self.root.lock().kill();
    }
}
#[test]
fn draft_last_alias_abort_and_started_child_survives_controller() {
    let _serial = SERIAL.lock().unwrap();
    let baseline = bitmap::count();
    {
        let f = Fixture::new();
        let control = f.create();
        let alias = f
            .caller
            .lock()
            .handle_table()
            .lock()
            .duplicate(control as u32)
            .unwrap() as usize;
        f.close(control);
        assert_eq!(f.root.lock().process_count(), 2);
        f.close(alias);
        assert_eq!(f.root.lock().process_count(), 1);
        let control = f.create();
        f.prepare(control);
        assert_eq!(f.call(21, [control, 0, 0, 0, 0, 0]), 0);
        assert_eq!(f.call(21, [control, 0, 0, 0, 0, 0]), -16);
        assert_eq!(f.call(22, [control, 0, 0, 0, 0, 0]), -16);
        f.close(control);
        assert_eq!(scheduler::count(), 1);
        assert_eq!(f.root.lock().process_count(), 2);
        scheduler::reap();
        assert_eq!(f.root.lock().process_count(), 1);
    }
    assert_eq!(bitmap::count(), baseline);
    assert_eq!(task::process::live_processes(), 0);
}
#[test]
fn mapping_rights_layout_sealing_and_guard_boundaries() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let control = f.create();
    use capabilities::{ObjectType, Rights};
    let memory = Arc::new(vmo::Vmo::create(4096).unwrap());
    let handle = f.cap(
        memory.clone(),
        ObjectType::Vmo,
        Rights::READ | Rights::WRITE | Rights::MAP | Rights::EXECUTE,
    );
    assert_eq!(f.map(control, handle, [0x10000, 0, 4096, 5, 0, 0]), -13);
    assert_eq!(f.map(control, handle, [0x10000, 0, 4096, 7, 0, 0]), -22);
    assert_eq!(f.map(control, handle, [0x10000, 0, 4096, 1, 1, 0]), -22);
    assert_eq!(
        f.map(
            control,
            handle,
            [kernel::syscall::USER_STACK as u64 - 4096, 0, 4096, 1, 0, 0]
        ),
        -22
    );
    assert_eq!(f.map(control, handle, [0x10000, 0, 4096, 3, 0, 0]), 0);
    assert_eq!(memory.seal(), Err(vmo::VmoError::Busy));
    assert_eq!(f.map(control, handle, [0x10000, 0, 4096, 1, 0, 0]), -17);
    assert_eq!(f.call(22, [control, 0, 0, 0, 0, 0]), 0);
    assert_eq!(memory.seal(), Ok(()));
    assert_eq!(f.call(22, [control, 0, 0, 0, 0, 0]), 0);
}
#[test]
fn delegation_is_a_move_rejects_graph_authority_and_revalidates_parent() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let control = f.create();
    use capabilities::{ObjectType, Rights};
    let forged = f.cap(
        Arc::new(capabilities::Resource),
        ObjectType::Job,
        Rights::MANAGE | Rights::TRANSFER,
    );
    assert_eq!(f.call(19, [control, forged, 0, 0, 0, 0]), -22);
    let memory = f.cap(
        Arc::new(vmo::Vmo::create(4096).unwrap()),
        ObjectType::Vmo,
        Rights::READ | Rights::TRANSFER,
    );
    assert_eq!(f.call(19, [control, memory, 0, 0, 0, 0]), 0);
    assert!(f
        .caller
        .lock()
        .handle_table()
        .lock()
        .lookup(memory as u32)
        .is_err());
    let node = {
        let p = f.caller.lock();
        let t = p.handle_table().lock();
        Arc::clone(t.lookup(f.job as u32).unwrap())
    };
    node.revoke();
    assert_eq!(f.call(21, [control, 0, 0, 0, 0, 0]), -116);
    assert_eq!(f.call(22, [control, 0, 0, 0, 0, 0]), 0);
}
#[test]
fn admission_failure_keeps_draft_retryable_and_drafts_count_toward_limit() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let control = f.create();
    f.prepare(control);
    scheduler::FAIL_START.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(f.call(21, [control, 0, 0, 0, 0, 0]), -12);
    assert_eq!(scheduler::count(), 0);
    scheduler::FAIL_START.store(false, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(f.call(21, [control, 0, 0, 0, 0, 0]), 0);
    let mut drafts = Vec::new();
    for _ in 2..64 {
        drafts.push(f.create());
    }
    assert_eq!(f.call(17, [f.job, 0, 0, 0, 0, 0]), -28);
    for handle in drafts {
        f.close(handle);
    }
    assert_eq!(task::process::live_processes(), 2);
}
#[test]
fn every_fallible_construction_step_releases_drafts_and_backing() {
    let _serial = SERIAL.lock().unwrap();
    for failure in 0..28 {
        let baseline = bitmap::count();
        {
            let f = Fixture::new();
            let count = capabilities::CAP_NODE_ALLOCATOR.live_count();
            FAIL.set(Some(failure));
            let result = f.call(17, [f.job, 0, 0, 0, 0, 0]);
            FAIL.set(None);
            if result >= 0 {
                f.close(result as usize);
            } else {
                assert_eq!(result, -12);
            }
            assert_eq!(f.root.lock().process_count(), 1);
            assert_eq!(capabilities::CAP_NODE_ALLOCATOR.live_count(), count);
        }
        assert_eq!(bitmap::count(), baseline);
        assert_eq!(task::process::live_processes(), 0);
    }
    for fail in 0..10 {
        let f = Fixture::new();
        let control = f.create();
        // Map executable backing before forcing initial-thread resource failures.
        use capabilities::{ObjectType, Rights};
        let code = Arc::new(vmo::Vmo::create(4096).unwrap());
        code.seal().unwrap();
        let code = f.cap(
            code,
            ObjectType::Vmo,
            Rights::READ | Rights::MAP | Rights::EXECUTE,
        );
        assert_eq!(f.map(control, code, [0x10000, 0, 4096, 5, 0, 0]), 0);
        let baseline = bitmap::count();
        FAIL.set(Some(fail));
        let result = f.call(20, [control, 0x10000, usize::MAX, 0, 0, 0]);
        FAIL.set(None);
        if result < 0 {
            assert_eq!(result, -12);
            assert_eq!(bitmap::count(), baseline);
            assert_eq!(f.call(20, [control, 0x10000, usize::MAX, 0, 0, 0]), 0);
        }
        assert_eq!(f.call(22, [control, 0, 0, 0, 0, 0]), 0);
    }
}

#[test]
fn explicit_channel_delegation_populates_slot_independent_startup() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let control = f.create();
    use capabilities::{ObjectType, Rights};
    let (endpoint, _peer) = channel::Channel::create().unwrap();
    let source = f.cap(
        endpoint,
        ObjectType::Channel,
        Rights::READ | Rights::WRITE | Rights::WAIT | Rights::TRANSFER,
    );
    let slot = f.call(19, [control, source, 0, 0, 0, 0]);
    assert!(slot >= 0);
    assert_eq!(f.call(20, [control, 0x10000, source, 0, 0, 0]), -14);
    f.prepare_bootstrap(control, slot as usize);
    assert_eq!(f.call(21, [control, 0, 0, 0, 0, 0]), 0);
    let child = scheduler::process();
    let child = child.lock();
    assert_eq!(child.handle_table().lock().count(), 1);
    let thread = child.threads()[0].lock();
    let stack = thread.user_stack().unwrap();
    let mut bytes = [0; startup::SIZE];
    stack
        .read_owned(stack.size() - startup::SIZE, &mut bytes)
        .unwrap();
    assert_eq!(
        startup::Startup::decode(&bytes).unwrap().bootstrap,
        Some(slot as u32)
    );
}
#[test]
fn killed_job_denies_mutation_and_physical_stack_failures_roll_back() {
    let _serial = SERIAL.lock().unwrap();
    for failure in 0..10 {
        let before = bitmap::count();
        {
            let f = Fixture::new();
            let control = f.create();
            use capabilities::{ObjectType, Rights};
            let code = Arc::new(vmo::Vmo::create(4096).unwrap());
            code.seal().unwrap();
            let code = f.cap(
                code,
                ObjectType::Vmo,
                Rights::READ | Rights::MAP | Rights::EXECUTE,
            );
            assert_eq!(f.map(control, code, [0x10000, 0, 4096, 5, 0, 0]), 0);
            let frames = bitmap::count();
            bitmap::FAIL_FRAME.store(failure, std::sync::atomic::Ordering::Relaxed);
            let result = f.call(20, [control, 0x10000, usize::MAX, 0, 0, 0]);
            bitmap::FAIL_FRAME.store(-1, std::sync::atomic::Ordering::Relaxed);
            if result < 0 {
                assert_eq!(result, -12);
                assert_eq!(bitmap::count(), frames);
            }
            f.root.lock().kill();
            assert_eq!(f.call(21, [control, 0, 0, 0, 0, 0]), -16);
            assert_eq!(f.call(22, [control, 0, 0, 0, 0, 0]), 0);
        }
        assert_eq!(bitmap::count(), before);
        assert_eq!(task::process::live_processes(), 0);
    }
}

#[test]
fn target_job_quota_is_enforced_without_spending_the_callers_budget() {
    let _serial = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let limited = f.root.lock().create_child_job("limited", 0).unwrap();
    let node = syscall_task::job_capability(&limited).unwrap();
    let authority = f.caller.lock().handle_table().lock().insert(node).unwrap() as usize;
    let control = f.call(17, [authority, 0, 0, 0, 0, 0]);
    assert!(control >= 0);
    use capabilities::{ObjectType, Rights};
    let code = Arc::new(vmo::Vmo::create(4096).unwrap());
    code.seal().unwrap();
    let code = f.cap(code, ObjectType::Vmo, Rights::READ | Rights::MAP | Rights::EXECUTE);
    assert_eq!(f.map(control as usize, code, [0x10000, 0, 4096, 5, 0, 0]), 0);
    let before = bitmap::count();
    assert_eq!(f.call(20, [control as usize, 0x10000, usize::MAX, 0, 0, 0]), -28);
    assert_eq!(bitmap::count(), before);
    assert_eq!(limited.lock().memory_used(), 0);
    assert_eq!(f.call(22, [control as usize, 0, 0, 0, 0, 0]), 0);
    assert_eq!(limited.lock().process_count(), 0);
}
