//! Production ledger and allocator with raw host-backed frames, not the host global allocator.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;
#[path = "../ferber/src/kernel/memory/accounting.rs"]
mod accounting;
#[path = "../ferber/src/kernel/memory/heap.rs"]
mod heap;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;
use accounting::{AllocError, BudgetRef, FrameCharge, JobBudget};
use std::alloc::{GlobalAlloc, Layout};
use std::sync::{LazyLock, Mutex};
static TEST: Mutex<()> = Mutex::new(());
static HEAP: heap::BuddyAllocator = heap::BuddyAllocator;
mod kernel {
    pub use crate::sync;
}
mod phys_map {
    pub fn phys_to_virt(frame: usize) -> Option<usize> {
        Some(frame)
    }
    pub fn virt_to_phys(address: usize) -> Option<usize> {
        Some(address)
    }
}
mod bitmap {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
    static FRAMES: LazyLock<Mutex<HashMap<usize, Layout>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    pub static FAIL_AFTER: AtomicIsize = AtomicIsize::new(-1);
    pub static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
    pub fn count() -> usize {
        FRAMES
            .lock()
            .unwrap()
            .values()
            .map(|layout| layout.size() / 4096)
            .sum()
    }
    pub fn allocate_frame() -> Option<usize> {
        allocate_frames(1)
    }
    pub fn allocate_frames(count: usize) -> Option<usize> {
        ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let left = FAIL_AFTER.load(Ordering::Relaxed);
        if left == 0 {
            return None;
        }
        if left > 0 {
            FAIL_AFTER.fetch_sub(1, Ordering::Relaxed);
        }
        let layout = Layout::from_size_align(count.checked_mul(4096)?, 4096).ok()?;
        let pointer = unsafe { std::alloc::alloc(layout) };
        if pointer.is_null() {
            return None;
        }
        FRAMES.lock().unwrap().insert(pointer as usize, layout);
        Some(pointer as usize)
    }
    pub fn free_frame(frame: usize) -> Result<(), ()> {
        free_frames(frame, 1)
    }
    pub fn free_frames(frame: usize, count: usize) -> Result<(), ()> {
        let mut frames = FRAMES.lock().unwrap();
        if frames.get(&frame).ok_or(())?.size() != count * 4096 {
            return Err(());
        }
        let layout = frames.remove(&frame).unwrap();
        unsafe {
            std::alloc::dealloc(frame as *mut u8, layout);
        }
        Ok(())
    }
}
fn job(parent: &BudgetRef, pages: usize) -> BudgetRef {
    JobBudget::child(parent, pages * 4096).unwrap()
}
fn allocate(owner: &BudgetRef, layout: Layout) -> Result<*mut u8, AllocError> {
    accounting::with_budget(owner, || {
        accounting::attempt(|| {
            let pointer = unsafe { HEAP.alloc(layout) };
            if pointer.is_null() {
                Err(())
            } else {
                Ok(pointer)
            }
        })
    })
}
fn weak(owner: &BudgetRef) -> std::sync::Weak<JobBudget> {
    match owner {
        BudgetRef::Job(owner) => std::sync::Arc::downgrade(owner),
        _ => panic!("expected child"),
    }
}
#[test]
fn ancestry_limits_close_lifetime_and_depth() {
    let _serial = TEST.lock().unwrap();
    let baseline = BudgetRef::system().used();
    let parent = job(&BudgetRef::system(), 2);
    let child = job(&parent, 4);
    let mut first = FrameCharge::reserve(&child, 1).unwrap();
    assert_eq!(parent.used(), 4096);
    let second = FrameCharge::reserve(&child, 1).unwrap();
    assert_eq!(
        FrameCharge::reserve(&child, 1).unwrap_err(),
        AllocError::Quota
    );
    assert_eq!(parent.used(), 8192);
    assert_eq!(child.used(), 8192);
    first.merge(second);
    assert_eq!(first.pages(), 2);
    assert_eq!(first.grow(usize::MAX), Err(AllocError::Quota));
    let parent_weak = weak(&parent);
    parent.close();
    assert!(child.is_closed());
    assert_eq!(
        FrameCharge::reserve(&child, 0).unwrap_err(),
        AllocError::Quota
    );
    drop(parent);
    drop(child);
    assert!(parent_weak.upgrade().is_some());
    first.release(1);
    assert_eq!(first.pages(), 1);
    drop(first);
    assert!(parent_weak.upgrade().is_none());
    assert_eq!(BudgetRef::system().used(), baseline);
    let mut chain = vec![BudgetRef::system()];
    for _ in 0..=accounting::MAX_JOB_DEPTH {
        chain.push(job(chain.last().unwrap(), 4));
    }
    assert!(matches!(
        JobBudget::child(chain.last().unwrap(), 4096),
        Err(AllocError::Quota)
    ));
    drop(chain);
}
#[test]
fn headers_alignment_pages_and_original_owner() {
    let _serial = TEST.lock().unwrap();
    let before = BudgetRef::system().used();
    let owner = job(&BudgetRef::system(), 512);
    let foreign = job(&BudgetRef::system(), 1);
    for align in [1, 8, 32, 64, 256, 4096] {
        for size in [1, 31, 32, 33, 128, 1024, 2048, 4064, 4096, 8192, 65_536] {
            let layout = Layout::from_size_align(size, align).unwrap();
            let pointer = allocate(&owner, layout).unwrap();
            assert_eq!(pointer as usize % align, 0);
            assert_eq!(owner.used(), bitmap::count() * 4096);
            unsafe {
                pointer.write_bytes(0x74, size);
            }
            accounting::with_budget(&foreign, || unsafe {
                HEAP.dealloc(pointer, layout);
            });
            assert_eq!(owner.used(), 0);
            assert_eq!(foreign.used(), 0);
            assert_eq!(bitmap::count(), 0);
        }
    }
    assert_eq!(BudgetRef::system().used(), before);
}
#[test]
fn per_owner_fragmentation_and_closed_arena() {
    let _serial = TEST.lock().unwrap();
    let a = job(&BudgetRef::system(), 1);
    let b = job(&BudgetRef::system(), 1);
    let layout = Layout::from_size_align(1, 1).unwrap();
    let p = allocate(&a, layout).unwrap();
    let q = allocate(&b, layout).unwrap();
    assert_eq!(a.used(), 4096);
    assert_eq!(b.used(), 4096);
    assert_eq!(bitmap::count(), 2);
    unsafe {
        q.write(0x55);
    }
    a.close();
    assert_eq!(allocate(&a, layout), Err(AllocError::Quota));
    unsafe {
        HEAP.dealloc(p, layout);
    }
    assert_eq!(a.used(), 0);
    assert_eq!(b.used(), 4096);
    assert_eq!(unsafe { q.read() }, 0x55);
    unsafe {
        HEAP.dealloc(q, layout);
    }
    assert_eq!(bitmap::count(), 0);
}
#[test]
fn coalescence_tracks_full_page_not_payload_bytes() {
    let _serial = TEST.lock().unwrap();
    let owner = job(&BudgetRef::system(), 4);
    let layout = Layout::from_size_align(32, 32).unwrap();
    let mut pointers = Vec::new();
    for _ in 0..128 {
        pointers.push(allocate(&owner, layout).unwrap());
    }
    assert_eq!(owner.used(), 8192);
    assert_eq!(bitmap::count(), 2);
    for pointer in pointers.drain(..).rev() {
        unsafe {
            HEAP.dealloc(pointer, layout);
        }
    }
    assert_eq!(owner.used(), 0);
    assert_eq!(bitmap::count(), 0);
}
#[test]
fn realloc_keeps_owner_counts_peak_and_preserves_failure() {
    let _serial = TEST.lock().unwrap();
    let layout = Layout::from_size_align(3000, 8).unwrap();
    let tight = job(&BudgetRef::system(), 2);
    let old = allocate(&tight, layout).unwrap();
    unsafe {
        old.write_bytes(0x38, 3000);
    }
    let grown = accounting::attempt(|| {
        let pointer = unsafe { HEAP.realloc(old, layout, 5000) };
        if pointer.is_null() {
            Err(())
        } else {
            Ok(pointer)
        }
    });
    assert_eq!(grown, Err(AllocError::Quota));
    assert_eq!(tight.used(), 4096);
    assert!(unsafe { core::slice::from_raw_parts(old, 3000) }
        .iter()
        .all(|b| *b == 0x38));
    unsafe {
        HEAP.dealloc(old, layout);
    }
    let roomy = job(&BudgetRef::system(), 3);
    let foreign = job(&BudgetRef::system(), 1);
    let old = allocate(&roomy, layout).unwrap();
    unsafe {
        old.write_bytes(0x19, 3000);
    }
    let grown = accounting::with_budget(&foreign, || unsafe { HEAP.realloc(old, layout, 5000) });
    assert!(!grown.is_null());
    assert_eq!(roomy.used(), 8192);
    assert_eq!(foreign.used(), 0);
    assert!(unsafe { core::slice::from_raw_parts(grown, 3000) }
        .iter()
        .all(|b| *b == 0x19));
    roomy.close();
    let shrink = unsafe { HEAP.realloc(grown, Layout::from_size_align(5000, 8).unwrap(), 100) };
    assert_eq!(shrink, grown);
    unsafe {
        HEAP.dealloc(shrink, Layout::from_size_align(100, 8).unwrap());
    }
    assert_eq!(roomy.used(), 0);
    assert_eq!(bitmap::count(), 0);
}
#[test]
fn failure_scope_unwind_and_requested_size_ceiling() {
    use std::sync::atomic::Ordering;
    let _serial = TEST.lock().unwrap();
    let no_space = job(&BudgetRef::system(), 0);
    let layout = Layout::from_size_align(128, 64).unwrap();
    assert_eq!(allocate(&no_space, layout), Err(AllocError::Quota));
    assert!(accounting::current().same_owner(&BudgetRef::system()));
    bitmap::FAIL_AFTER.store(0, Ordering::Relaxed);
    let owner = job(&BudgetRef::system(), usize::MAX / 4096);
    assert_eq!(allocate(&owner, layout), Err(AllocError::OutOfMemory));
    assert_eq!(owner.used(), 0);
    let attempts = bitmap::ATTEMPTS.load(Ordering::Relaxed);
    assert_eq!(
        allocate(
            &owner,
            Layout::from_size_align(heap::MAX_ALLOC_SIZE + 1, 1).unwrap()
        ),
        Err(AllocError::OutOfMemory)
    );
    assert_eq!(bitmap::ATTEMPTS.load(Ordering::Relaxed), attempts);
    assert_eq!(
        allocate(
            &owner,
            Layout::from_size_align(heap::MAX_ALLOC_SIZE, 4096).unwrap()
        ),
        Err(AllocError::OutOfMemory)
    );
    assert_eq!(bitmap::ATTEMPTS.load(Ordering::Relaxed), attempts + 1);
    bitmap::FAIL_AFTER.store(-1, Ordering::Relaxed);
    assert_eq!(
        accounting::attempt(|| Err::<(), _>(())),
        Err(AllocError::OutOfMemory)
    );
    assert_eq!(owner.used(), 0);
    assert_eq!(bitmap::count(), 0);
}
#[test]
fn allocation_context_is_owned_and_scopes_restore_on_unwind() {
    let _serial = TEST.lock().unwrap();
    let owner = job(&BudgetRef::system(), 2);
    let reference = weak(&owner);
    accounting::set_current(&owner);
    drop(owner);
    assert!(reference.upgrade().is_some());
    let foreign = job(&BudgetRef::system(), 2);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        accounting::with_budget(&foreign, || {
            assert!(accounting::current().same_owner(&foreign));
            accounting::assert_switch_safe();
        });
    }));
    assert!(result.is_err());
    assert!(!accounting::current().same_owner(&foreign));
    accounting::assert_switch_safe();
    accounting::set_system();
    assert!(reference.upgrade().is_none());
}

#[test]
fn live_header_retains_dead_owner_and_closed_ancestor_blocks_cached_space() {
    let _serial = TEST.lock().unwrap();
    let parent = job(&BudgetRef::system(), 2);
    let owner = job(&parent, 1);
    let owner_weak = weak(&owner);
    let parent_weak = weak(&parent);
    let layout = Layout::from_size_align(1, 1).unwrap();
    let pointer = allocate(&owner, layout).unwrap();
    parent.close();
    assert_eq!(allocate(&owner, layout), Err(AllocError::Quota));
    drop(parent);
    drop(owner);
    assert!(owner_weak.upgrade().is_some());
    assert!(parent_weak.upgrade().is_some());
    unsafe {
        HEAP.dealloc(pointer, layout);
    }
    assert!(owner_weak.upgrade().is_none());
    assert!(parent_weak.upgrade().is_none());
    assert_eq!(bitmap::count(), 0);
}

#[test]
fn concurrent_ancestry_and_allocator_admission_never_overcommit() {
    use std::sync::Barrier;
    let _serial = TEST.lock().unwrap();
    let owner = job(&BudgetRef::system(), 4);
    let gate = std::sync::Arc::new(Barrier::new(32));
    let workers: Vec<_> = (0..32)
        .map(|byte| {
            let owner = owner.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                let layout = Layout::from_size_align(3000, 8).unwrap();
                let pointer = allocate(&owner, layout);
                if let Ok(pointer) = pointer {
                    unsafe {
                        pointer.write_bytes(byte, 3000);
                    }
                }
                gate.wait();
                assert_eq!(owner.used(), 4 * 4096);
                gate.wait();
                if let Ok(pointer) = pointer {
                    assert!(unsafe { core::slice::from_raw_parts(pointer, 3000) }
                        .iter()
                        .all(|value| *value == byte));
                    unsafe {
                        HEAP.dealloc(pointer, layout);
                    }
                    true
                } else {
                    assert_eq!(pointer, Err(AllocError::Quota));
                    false
                }
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>(),
        4
    );
    assert_eq!(owner.used(), 0);
    assert_eq!(bitmap::count(), 0);
}

#[test]
fn realloc_ceiling_applies_even_when_header_padding_has_room() {
    let _serial = TEST.lock().unwrap();
    let owner = job(&BudgetRef::system(), heap::MAX_ALLOC_SIZE / 4096 + 2);
    let layout = Layout::from_size_align(heap::MAX_ALLOC_SIZE, 8).unwrap();
    let pointer = allocate(&owner, layout).unwrap();
    unsafe {
        pointer.write(0x74);
    }
    let replacement = accounting::attempt(|| {
        let pointer = unsafe { HEAP.realloc(pointer, layout, heap::MAX_ALLOC_SIZE + 1) };
        if pointer.is_null() {
            Err(())
        } else {
            Ok(pointer)
        }
    });
    assert_eq!(replacement, Err(AllocError::OutOfMemory));
    assert_eq!(unsafe { pointer.read() }, 0x74);
    unsafe {
        HEAP.dealloc(pointer, layout);
    }
    assert_eq!(owner.used(), 0);
    assert_eq!(bitmap::count(), 0);
}
