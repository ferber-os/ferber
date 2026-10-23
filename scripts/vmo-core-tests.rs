//! Exercise production VMO allocation, charging, locking and copy preflight.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;
#[path = "../ferber/src/kernel/memory/vmo.rs"]
pub mod vmo;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static FAIL_ALLOCATION: Cell<bool> = const { Cell::new(false) };
}
struct TestAllocator;
unsafe impl GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_ALLOCATION.try_with(Cell::get).unwrap_or(false) {
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
    pub use crate::sync;
    pub mod capabilities {
        pub trait KernelObject: Send + Sync { fn type_name(&self) -> &'static str; }
        #[derive(Clone, Copy)]
        pub struct Rights(u32);
        impl Rights {
            pub const MAP: Self = Self(1);
            pub const READ: Self = Self(2);
            pub const WRITE: Self = Self(4);
            pub const EXECUTE: Self = Self(8);
            pub fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
        }
    }
    pub mod memory {
        pub mod paging {
            pub struct MapFlags { pub readable: bool, pub writable: bool, pub executable: bool }
        }
        pub mod phys_map {
            use std::sync::atomic::{AtomicUsize, Ordering};
            pub static DENIED: AtomicUsize = AtomicUsize::new(0);
            pub fn phys_to_virt(frame: usize) -> Option<usize> {
                (frame != DENIED.load(Ordering::Relaxed)).then_some(frame)
            }
            pub fn mmio_to_virt(_: usize) -> Option<usize> { None }
        }
        pub mod bitmap {
            use std::{alloc::{alloc, dealloc, Layout}, collections::HashMap,
                sync::{Mutex, LazyLock, atomic::{AtomicIsize, Ordering}}};
            pub static FAIL_AFTER: AtomicIsize = AtomicIsize::new(-1);
            static FRAMES: LazyLock<Mutex<HashMap<usize, Layout>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
            pub fn count() -> usize { FRAMES.lock().unwrap().len() }
            pub fn allocate_frame() -> Option<usize> { allocate_frames(1) }
            pub fn allocate_frames(count: usize) -> Option<usize> {
                if FAIL_AFTER.try_update(Ordering::Relaxed, Ordering::Relaxed,
                    |left| if left < 0 { Some(left) } else { left.checked_sub(1).filter(|next| *next >= 0) }).is_err() {
                    return None;
                }
                let layout = Layout::from_size_align(count * 4096, 4096).unwrap();
                let ptr = unsafe { alloc(layout) };
                if ptr.is_null() { return None; }
                unsafe { ptr.write_bytes(0xaa, layout.size()); }
                FRAMES.lock().unwrap().insert(ptr as usize, layout);
                Some(ptr as usize)
            }
            pub fn free_frame(frame: usize) -> Result<(), ()> {
                let layout = FRAMES.lock().unwrap().remove(&frame).ok_or(())?;
                unsafe { dealloc(frame as *mut u8, layout); }
                Ok(())
            }
        }
    }
}

#[test]
fn budget_calculation_includes_zero_and_caps() {
    assert_eq!(vmo::user_budget_limit(0), 0);
    assert_eq!(vmo::user_budget_limit(3), 0);
    assert_eq!(vmo::user_budget_limit(4), 1);
    assert_eq!(vmo::user_budget_limit(65_535), 16_383);
    assert_eq!(vmo::user_budget_limit(65_536), 16_384);
    assert_eq!(vmo::user_budget_limit(usize::MAX), 16_384);
}

#[test]
fn production_backing_and_budget_lifecycle() {
    use kernel::memory::{bitmap, phys_map};
    use std::sync::{Arc, Barrier, atomic::Ordering};
    use vmo::{Vmo, VmoError};
    vmo::init_user_budget(64);
    FAIL_ALLOCATION.set(true);
    let metadata_failure = Vmo::create_user(8192);
    FAIL_ALLOCATION.set(false);
    assert!(matches!(metadata_failure, Err(VmoError::OutOfMemory)));
    assert_eq!(bitmap::count(), 0);
    assert_eq!(vmo::user_budget_stats(), (16, 0));
    let before_publication = Vmo::create_user(8192).unwrap();
    assert_eq!(bitmap::count(), 2);
    assert_eq!(vmo::user_budget_stats().1, 2);
    FAIL_ALLOCATION.set(true);
    let object_failure = Arc::try_new(before_publication);
    FAIL_ALLOCATION.set(false);
    assert!(object_failure.is_err());
    assert_eq!(bitmap::count(), 0);
    assert_eq!(vmo::user_budget_stats(), (16, 0));
    assert!(matches!(Vmo::create_user(0), Err(VmoError::ZeroSize)));
    assert!(matches!(Vmo::create_user(vmo::MAX_USER_VMO_SIZE + 1), Err(VmoError::TooLarge)));
    bitmap::FAIL_AFTER.store(1, Ordering::Relaxed);
    assert!(matches!(Vmo::create_user(8192), Err(VmoError::OutOfMemory)));
    assert_eq!(bitmap::count(), 0);
    assert_eq!(vmo::user_budget_stats(), (16, 0));
    bitmap::FAIL_AFTER.store(-1, Ordering::Relaxed);
    let memory = Arc::new(Vmo::create_user(4097).unwrap());
    assert_eq!(vmo::user_budget_stats(), (16, 2));
    for frame in memory.frames().iter() {
        let bytes = unsafe { std::slice::from_raw_parts(frame as *const u8, 4096) };
        assert!(bytes.iter().all(|byte| *byte == 0));
    }
    assert_eq!(memory.validate_owned_range(4098, 0), Err(VmoError::OutOfBounds));
    assert_eq!(memory.validate_owned_range(4097, 0), Ok(()));
    assert_eq!(memory.validate_owned_range(usize::MAX, 2), Err(VmoError::OutOfBounds));
    let alias = memory.clone();
    alias.write_owned(4094, &[1, 2, 3]).unwrap();
    let mut output = [0; 3];
    memory.read_owned(4094, &mut output).unwrap();
    assert_eq!(output, [1, 2, 3]);
    phys_map::DENIED.store(memory.frames().get(1).unwrap(), Ordering::Relaxed);
    assert_eq!(memory.write_owned(4094, &[7, 8, 9]), Err(VmoError::InvalidBacking));
    let mut untouched = [55; 3];
    assert_eq!(memory.read_owned(4094, &mut untouched), Err(VmoError::InvalidBacking));
    assert_eq!(untouched, [55; 3]);
    phys_map::DENIED.store(0, Ordering::Relaxed);
    memory.read_owned(4094, &mut output).unwrap();
    assert_eq!(output, [1, 2, 3]);
    let physical = Vmo::create_physical(memory.frames().get(0).unwrap(), 4096).unwrap();
    assert_eq!(physical.read_owned(0, &mut []), Err(VmoError::InvalidBacking));
    drop(physical);
    drop(memory);
    assert_eq!(vmo::user_budget_stats().1, 2);
    drop(alias);
    assert_eq!(bitmap::count(), 0);
    assert_eq!(vmo::user_budget_stats().1, 0);
    let shared = Arc::new(Vmo::create_user(8192).unwrap());
    let writers: Vec<_> = (1..=4).map(|value| {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let bytes = vec![value; 8192];
            let mut snapshot = vec![0; 8192];
            for _ in 0..50 {
                shared.write_owned(0, &bytes).unwrap();
                shared.read_owned(0, &mut snapshot).unwrap();
                assert!(snapshot.iter().all(|byte| *byte == snapshot[0]));
            }
        })
    }).collect();
    for writer in writers { writer.join().unwrap(); }
    drop(shared);
    let full = Vmo::create_user(16 * 4096).unwrap();
    assert!(matches!(Vmo::create_user(1), Err(VmoError::UserBudget)));
    let kernel_owned = Vmo::create(4096).unwrap();
    assert_eq!(vmo::user_budget_stats().1, 16);
    drop(kernel_owned);
    drop(full);
    let barrier = Arc::new(Barrier::new(32));
    let workers: Vec<_> = (0..32).map(|_| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let result = Vmo::create_user(1);
            barrier.wait();
            assert!(vmo::user_budget_stats().1 <= 16);
            barrier.wait();
            result.is_ok()
        })
    }).collect();
    assert_eq!(workers.into_iter().filter_map(|worker| worker.join().ok()).filter(|ok| *ok).count(), 16);
    assert_eq!(vmo::user_budget_stats().1, 0);
    assert_eq!(bitmap::count(), 0);
}
