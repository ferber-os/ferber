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
        pub struct Process {
            pub table: SpinLock<HandleTable>,
        }
        impl Process {
            pub fn handle_table(&self) -> &SpinLock<HandleTable> {
                &self.table
            }
            pub fn address_space(&self) -> &Self {
                self
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
            pub struct MapFlags {
                pub readable: bool,
                pub writable: bool,
                pub executable: bool,
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
}
