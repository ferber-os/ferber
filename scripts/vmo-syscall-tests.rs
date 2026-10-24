//! Exercise production VMO allocation, charging, locking and copy preflight.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;
#[path = "../ferber/src/kernel/capabilities/mod.rs"]
pub mod capabilities;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;
#[path = "../ferber/src/kernel/syscall_vmo.rs"]
mod syscall_vmo;
const SYS_VMO_CREATE: usize = 11;
const SYS_VMO_READ: usize = 12;
const SYS_VMO_WRITE: usize = 13;
const SYS_VMO_MAP: usize = 14;
const SYS_VMO_UNMAP: usize = 15;
fn cap_errno(error: capabilities::CapError) -> isize {
    use capabilities::CapError::*;
    match error {
        InvalidHandle => -9,
        InsufficientRights => -13,
        TableFull => -24,
        Revoked => -116,
        InvalidRights => -22,
        OutOfMemory => -12,
        NodeCapacity => -28,
    }
}
#[path = "../ferber/src/kernel/memory/vmo.rs"]
pub mod vmo;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static FAIL_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
    static FILL_ON_ALLOCATION: Cell<*const sync::SpinLock<kernel::task::Process>> = const { Cell::new(std::ptr::null()) };
}
struct TestAllocator;
unsafe impl GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_AFTER
            .try_with(|left| match left.get() {
                Some(0) => {
                    left.set(None);
                    let fill = FILL_ON_ALLOCATION.with(|cell| cell.replace(std::ptr::null()));
                    if fill.is_null() {
                        true
                    } else {
                        let process = unsafe { &*fill }.lock();
                        let mut table = process.table.lock();
                        let node = alloc::sync::Arc::clone(table.lookup(0).unwrap());
                        table.insert(node).unwrap();
                        false
                    }
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
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: TestAllocator = TestAllocator;

mod kernel {
    pub use crate::capabilities;
    pub use crate::sync;
    pub mod ipc {
        pub mod channel {
            pub struct Channel;
        }
    }
    pub mod task {
        use crate::{capabilities::HandleTable, sync::SpinLock};
        pub mod process {
            use crate::{vmo::Vmo, kernel::memory::paging::MapFlags};
            use alloc::sync::Arc;
            pub struct MappingRecord {
                pub base: usize, pub len: usize, pub offset: usize,
                pub flags: MapFlags, pub object: Arc<Vmo>,
            }
        }
        pub struct Process {
            pub table: SpinLock<HandleTable>,
            pub space: super::memory::paging::Space,
            pub mappings: Vec<process::MappingRecord>,
            pub guards: Vec<(usize, usize)>,
        }
        impl Process {
            pub fn handle_table(&self) -> &SpinLock<HandleTable> {
                &self.table
            }
            pub fn address_space(&self) -> &super::memory::paging::Space { &self.space }
            pub fn address_space_mut(&mut self) -> &mut super::memory::paging::Space { &mut self.space }
            pub fn mappings(&self) -> &[process::MappingRecord] { &self.mappings }
            pub fn mappings_mut(&mut self) -> &mut Vec<process::MappingRecord> { &mut self.mappings }
            pub fn mapping_reserved(&self, base: usize, end: usize) -> bool {
                self.guards.iter().any(|&(start, len)| base < start + len && start < end)
            }
        }
    }
    pub mod memory {
        pub use crate::vmo;
        pub mod user_mem {
            #[derive(Debug)]
            pub enum OutputError {
                UserMemory,
                Overlap,
            }
            pub fn preflight(_: &impl Sized, start: usize, len: usize, _: bool) -> Result<(), ()> {
                if start == 0 && len != 0 {
                    Err(())
                } else {
                    Ok(())
                }
            }
            pub fn validate_outputs(
                space: &impl Sized,
                spans: &[(usize, usize)],
            ) -> Result<(), OutputError> {
                for &(start, len) in spans {
                    preflight(space, start, len, true).map_err(|_| OutputError::UserMemory)?;
                }
                Ok(())
            }
            pub unsafe fn copy_from_user(
                _: &impl Sized,
                dst: &mut [u8],
                source: usize,
            ) -> Result<(), ()> {
                unsafe {
                    std::ptr::copy_nonoverlapping(source as *const u8, dst.as_mut_ptr(), dst.len());
                }
                Ok(())
            }
            pub unsafe fn copy_to_user(
                _: &impl Sized,
                dst: usize,
                source: &[u8],
            ) -> Result<(), ()> {
                unsafe {
                    std::ptr::copy_nonoverlapping(source.as_ptr(), dst as *mut u8, source.len());
                }
                Ok(())
            }
        }
        pub mod paging {
            use std::collections::BTreeMap;
            use crate::vmo::VmoFrames;
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct MapFlags {
                pub readable: bool, pub writable: bool, pub executable: bool, pub user: bool,
            }
            impl MapFlags {
                pub fn user_ro() -> Self { Self { readable: true, writable: false, executable: false, user: true } }
                pub fn user_rw() -> Self { Self { writable: true, ..Self::user_ro() } }
            }
            #[derive(Debug, PartialEq, Eq)]
            pub enum PageError { OutOfMemory, TableBudget, AlreadyMapped, NotMapped }
            #[derive(Default)]
            pub struct Space(pub BTreeMap<usize, (usize, MapFlags)>);
            pub trait AddressSpace {
                fn check_free_range(&self, base: usize, count: usize) -> Result<(), PageError>;
                fn query(&self, base: usize) -> Option<(usize, MapFlags)>;
                unsafe fn map_pages(&mut self, base: usize, frames: &VmoFrames, first: usize, count: usize, flags: MapFlags) -> Result<(), PageError>;
                unsafe fn unmap_pages_exact(&mut self, base: usize, frames: &VmoFrames, first: usize, count: usize, flags: MapFlags) -> Result<(), PageError>;
            }
            impl AddressSpace for Space {
                fn check_free_range(&self, base: usize, count: usize) -> Result<(), PageError> {
                    if (0..count).any(|i| self.0.contains_key(&(base + i * 4096))) { Err(PageError::AlreadyMapped) } else { Ok(()) }
                }
                fn query(&self, base: usize) -> Option<(usize, MapFlags)> { self.0.get(&base).copied() }
                unsafe fn map_pages(&mut self, base: usize, frames: &VmoFrames, first: usize, count: usize, flags: MapFlags) -> Result<(), PageError> {
                    self.check_free_range(base, count)?;
                    for i in 0..count { self.0.insert(base + i * 4096, (frames.get(first+i).unwrap(), flags)); }
                    Ok(())
                }
                unsafe fn unmap_pages_exact(&mut self, base: usize, frames: &VmoFrames, first: usize, count: usize, flags: MapFlags) -> Result<(), PageError> {
                    if (0..count).any(|i| self.0.get(&(base + i * 4096)) != Some(&(frames.get(first+i).unwrap(), flags))) { return Err(PageError::NotMapped); }
                    for i in 0..count { self.0.remove(&(base + i * 4096)); }
                    Ok(())
                }
            }
        }
        pub mod phys_map {
            use std::sync::atomic::{AtomicUsize, Ordering};
            pub static DENIED: AtomicUsize = AtomicUsize::new(0);
            pub fn phys_to_virt(frame: usize) -> Option<usize> {
                (frame != DENIED.load(Ordering::Relaxed)).then_some(frame)
            }
            pub fn mmio_to_virt(_: usize) -> Option<usize> {
                None
            }
        }
        pub mod bitmap {
            use std::{
                alloc::{alloc, dealloc, Layout},
                collections::HashMap,
                sync::{
                    atomic::{AtomicIsize, Ordering},
                    LazyLock, Mutex,
                },
            };
            pub static FAIL_AFTER: AtomicIsize = AtomicIsize::new(-1);
            static FRAMES: LazyLock<Mutex<HashMap<usize, Layout>>> =
                LazyLock::new(|| Mutex::new(HashMap::new()));
            pub fn count() -> usize {
                FRAMES.lock().unwrap().len()
            }
            pub fn allocate_frame() -> Option<usize> {
                allocate_frames(1)
            }
            pub fn allocate_frames(count: usize) -> Option<usize> {
                if FAIL_AFTER
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        if left < 0 {
                            Some(left)
                        } else {
                            left.checked_sub(1).filter(|next| *next >= 0)
                        }
                    })
                    .is_err()
                {
                    return None;
                }
                let layout = Layout::from_size_align(count * 4096, 4096).unwrap();
                let ptr = unsafe { alloc(layout) };
                if ptr.is_null() {
                    return None;
                }
                unsafe {
                    ptr.write_bytes(0xaa, layout.size());
                }
                FRAMES.lock().unwrap().insert(ptr as usize, layout);
                Some(ptr as usize)
            }
            pub fn free_frame(frame: usize) -> Result<(), ()> {
                let layout = FRAMES.lock().unwrap().remove(&frame).ok_or(())?;
                unsafe {
                    dealloc(frame as *mut u8, layout);
                }
                Ok(())
            }
        }
    }
}

#[test]
fn production_syscall_allocation_rollback() {
    use alloc::sync::Arc;
    use capabilities::{HandleTable, CAP_NODE_ALLOCATOR};
    use kernel::{memory::bitmap, task::Process};
    vmo::init_user_budget(65536);
    let process = Arc::new(sync::SpinLock::new(Process {
        table: sync::SpinLock::new(HandleTable::new()),
        space: Default::default(), mappings: Vec::new(), guards: Vec::new(),
    }));
    // Warm host-only frame bookkeeping before injecting production allocations.
    drop(vmo::Vmo::create(4096).unwrap());
    let baseline = vmo::user_budget_stats();
    let mut failures = 0;
    let handle = loop {
        FAIL_AFTER.with(|left| left.set(Some(failures)));
        let result = syscall_vmo::dispatch(&process, SYS_VMO_CREATE, [4096, 0, 0, 0, 0, 0]);
        FAIL_AFTER.with(|left| left.set(None));
        if result >= 0 {
            break result as u32;
        }
        assert_eq!(result, -12);
        assert_eq!(bitmap::count(), 0);
        assert_eq!(CAP_NODE_ALLOCATOR.live_count(), 0);
        assert_eq!(vmo::user_budget_stats(), baseline);
        assert_eq!(process.lock().table.lock().count(), 0);
        failures += 1;
        assert!(failures < 12);
    };
    assert!(
        failures >= 4,
        "frame-vector, frame, object, and node allocations"
    );
    let mut buffer = [0x77; 8];
    for nr in [SYS_VMO_READ, SYS_VMO_WRITE] {
        FAIL_AFTER.with(|left| left.set(Some(0)));
        let result = syscall_vmo::dispatch(
            &process,
            nr,
            [handle as usize, 0, buffer.as_mut_ptr() as usize, 8, 0, 0],
        );
        FAIL_AFTER.with(|left| left.set(None));
        assert_eq!(result, -12);
        assert_eq!(buffer, [0x77; 8]);
        let retained = {
            let p = process.lock();
            let t = p.table.lock();
            Arc::clone(t.lookup(handle).unwrap())
        };
        let mut data = [0; 8];
        retained
            .object_ref::<vmo::Vmo>()
            .unwrap()
            .read_owned(0, &mut data)
            .unwrap();
        assert_eq!(data, [0; 8]);
    }
    // Capacity can disappear after the early precheck: fail publication after allocating backing.
    let filler = {
        let p = process.lock();
        let t = p.table.lock();
        Arc::clone(t.lookup(handle).unwrap())
    };
    {
        let p = process.lock();
        let mut table = p.table.lock();
        while table.count() < 255 {
            table.insert(Arc::clone(&filler)).unwrap();
        }
    }
    FILL_ON_ALLOCATION.with(|cell| cell.set(Arc::as_ptr(&process)));
    FAIL_AFTER.with(|left| left.set(Some(failures - 1)));
    let result = syscall_vmo::dispatch(&process, SYS_VMO_CREATE, [4096, 0, 0, 0, 0, 0]);
    FAIL_AFTER.with(|left| left.set(None));
    FILL_ON_ALLOCATION.with(|cell| cell.set(std::ptr::null()));
    assert_eq!(result, -24);
    assert_eq!(bitmap::count(), 1);
    assert_eq!(vmo::user_budget_stats().1, baseline.1 + 1);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), 1);
    for handle in 1..256 {
        let removed = process.lock().table.lock().remove(handle).unwrap();
        drop(removed);
    }
    // Exhaust capability-node permits independently of backing allocation.
    let mut owners = Vec::new();
    for _ in 1..capabilities::NODE_CAPACITY {
        owners.push(
            CAP_NODE_ALLOCATOR
                .alloc_root(
                    Arc::new(capabilities::Resource),
                    capabilities::ObjectType::Resource,
                    syscall_vmo::INITIAL_RIGHTS,
                )
                .unwrap(),
        );
    }
    assert_eq!(
        syscall_vmo::dispatch(&process, SYS_VMO_CREATE, [4096, 0, 0, 0, 0, 0]),
        -28
    );
    assert_eq!(bitmap::count(), 1);
    assert_eq!(vmo::user_budget_stats().1, baseline.1 + 1);
    drop(owners);
    drop(filler);
    let node = process.lock().table.lock().remove(handle).unwrap();
    drop(node);
    assert_eq!(bitmap::count(), 0);
    assert_eq!(vmo::user_budget_stats(), baseline);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), 0);
    // Actual dispatcher: metadata OOM precedes PTE publication and releases ownership.
    let mapped = syscall_vmo::dispatch(&process, SYS_VMO_CREATE, [8193, 1, 0, 0, 0, 0]);
    assert!(mapped >= 0);
    const BASE: usize = 0x100000;
    let call_map = |offset, base, len, perms| syscall_vmo::dispatch(&process, SYS_VMO_MAP, [mapped as usize, offset, base, len, perms, 0]);
    process.lock().guards.push((BASE + 0x80000, 8192));
    assert_eq!(call_map(0, BASE + 0x80000, 4096, 1), -22);
    assert_eq!(call_map(8192, BASE, 4096, 1), -22);
    assert_eq!(call_map(0, BASE, 4096, 5), -22);
    FAIL_AFTER.with(|left| left.set(Some(0)));
    assert_eq!(call_map(0, BASE, 4096, 3), -12);
    FAIL_AFTER.with(|left| left.set(None));
    assert!(process.lock().space.0.is_empty());
    assert!(process.lock().mappings.is_empty());
    assert_eq!(call_map(0, BASE, 4096, 3), 0);
    assert_eq!(call_map(4096, BASE + 8192, 4096, 1), 0);
    assert_eq!(call_map(0, BASE, 4096, 1), -17);
    assert_eq!(syscall_vmo::dispatch(&process, SYS_VMO_UNMAP, [BASE, 8192, 0, 0, 0, 0]), -22);
    assert_eq!(syscall_vmo::dispatch(&process, SYS_VMO_UNMAP, [0, 4096, 0, 0, 0, 0]), -14);
    assert_eq!(syscall_vmo::dispatch(&process, SYS_VMO_UNMAP, [1usize << 47, 4096, 0, 0, 0, 0]), -14);
    {
        let mut p = process.lock();
        let raw = *p.space.0.get(&BASE).unwrap();
        p.space.0.insert(BASE + 0x40000, raw);
        while p.mappings.len() < 64 {
            let source = &p.mappings[0];
            let record = kernel::task::process::MappingRecord {
                base: source.base, len: source.len, offset: source.offset,
                flags: source.flags, object: Arc::clone(&source.object),
            };
            p.mappings.push(record);
        }
    }
    assert_eq!(call_map(0, BASE + 0x40000, 4096, 1), -17);
    process.lock().mappings.truncate(2);
    process.lock().space.0.remove(&(BASE + 0x40000));
    let pte = process.lock().space.0.remove(&BASE).unwrap();
    assert_eq!(syscall_vmo::dispatch(&process, SYS_VMO_UNMAP, [BASE, 4096, 0, 0, 0, 0]), -22);
    assert_eq!(process.lock().mappings.len(), 2);
    assert!(process.lock().space.0.contains_key(&(BASE + 8192)));
    process.lock().space.0.insert(BASE, pte);
    let node = process.lock().table.lock().remove(mapped as u32).unwrap();
    node.revoke();
    drop(node);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), 0);
    assert_eq!(bitmap::count(), 3);
    assert_eq!(syscall_vmo::dispatch(&process, SYS_VMO_UNMAP, [BASE, 4096, 0, 0, 0, 0]), 0);
    assert_eq!(bitmap::count(), 3);
    assert_eq!(syscall_vmo::dispatch(&process, SYS_VMO_UNMAP, [BASE + 8192, 4096, 0, 0, 0, 0]), 0);
    assert_eq!(bitmap::count(), 0);
    assert_eq!(vmo::user_budget_stats(), baseline);

    // Four aliases charge 16,384 mapped pages while retaining one 4,096-page VMO.
    const LARGE: usize = 16 * 1024 * 1024;
    const ALIASES: usize = 0x1000_0000;
    let large = syscall_vmo::dispatch(&process, SYS_VMO_CREATE, [LARGE, 1, 0, 0, 0, 0]);
    assert!(large >= 0);
    let map_alias = |base, len| syscall_vmo::dispatch(&process, SYS_VMO_MAP, [large as usize, 0, base, len, 3, 0]);
    let unmap_alias = |base| syscall_vmo::dispatch(&process, SYS_VMO_UNMAP, [base, LARGE, 0, 0, 0, 0]);
    for i in 0..4 {
        assert_eq!(map_alias(ALIASES + i * LARGE, LARGE), 0);
    }
    let charged = vmo::user_budget_stats();
    assert_eq!(charged.1, baseline.1 + 4096);
    assert_eq!(bitmap::count(), 4096);
    let snapshot = process.lock().space.0.clone();
    assert_eq!(snapshot.len(), 16384);
    assert_eq!(process.lock().mappings.iter().map(|record| record.len / 4096).sum::<usize>(), 16384);
    assert_eq!(map_alias(ALIASES + 4 * LARGE, 4096), -28);
    assert_eq!(process.lock().space.0, snapshot);
    assert_eq!(process.lock().mappings.len(), 4);
    assert_eq!(bitmap::count(), 4096);
    assert_eq!(vmo::user_budget_stats(), charged);
    assert_eq!(unmap_alias(ALIASES), 0);
    assert_eq!(map_alias(ALIASES + 4 * LARGE, LARGE), 0);
    assert_eq!(process.lock().space.0.len(), 16384);
    assert_eq!(process.lock().mappings.len(), 4);
    assert_eq!(bitmap::count(), 4096);
    assert_eq!(vmo::user_budget_stats(), charged);
    for i in 1..=4 {
        assert_eq!(unmap_alias(ALIASES + i * LARGE), 0);
    }
    drop(process.lock().table.lock().remove(large as u32).unwrap());
    assert!(process.lock().space.0.is_empty());
    assert!(process.lock().mappings.is_empty());
    assert_eq!(bitmap::count(), 0);
    assert_eq!(vmo::user_budget_stats(), baseline);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), 0);

}
