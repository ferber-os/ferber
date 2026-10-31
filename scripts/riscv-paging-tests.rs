//! Production Sv39 walkers with only physical allocation and privileged instructions mocked.
#![allow(dead_code)]
#![feature(allocator_ext)]
extern crate alloc;
#[path = "../ferber/src/kernel/sync.rs"] pub mod sync;
// The production ledger is exercised; host metadata uses the host allocator.
mod heap {
    pub struct Arena;
    impl Arena { pub const fn new() -> Self { Self } pub fn is_empty(&self) -> bool { true } }
}
#[path = "../ferber/src/kernel/memory/accounting.rs"] pub mod accounting;
std::thread_local! {
    static CHARGE_OBSERVER: std::cell::RefCell<Option<accounting::BudgetRef>> = const { std::cell::RefCell::new(None) };
}
static FREE_CHARGES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
fn observe_charge_before_free() {
    CHARGE_OBSERVER.with(|observer| {
        if let Some(owner) = observer.borrow().as_ref() {
            FREE_CHARGES.lock().unwrap().push(owner.used());
        }
    });
}


#[path = "../ferber/src/arch/riscv64/paging.rs"]
mod sv39;
#[path = "../ferber/src/kernel/memory/paging.rs"]
pub mod paging;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static FAIL_METADATA: Cell<bool> = const { Cell::new(false) }; }
struct TestAllocator;
unsafe impl GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_METADATA.try_with(Cell::get).unwrap_or(false) { std::ptr::null_mut() }
        else { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) { System.dealloc(pointer, layout); }
}
#[global_allocator]
static ALLOCATOR: TestAllocator = TestAllocator;
mod arch { pub type ArchAddressSpace = crate::sv39::Sv39AddressSpace; }
mod kernel {
    pub use crate::sync;
    pub mod memory {
        pub use crate::accounting;
        pub use crate::paging;
        pub mod vmo {
            pub struct VmoFrames(Vec<usize>);
            impl VmoFrames {
                pub fn new(frames: Vec<usize>) -> Self { Self(frames) }
                pub fn contiguous(base: usize, count: usize) -> Self {
                    Self((0..count).map(|i| base + i * 4096).collect())
                }
                pub fn count(&self) -> usize { self.0.len() }
                pub fn get(&self, index: usize) -> Option<usize> { self.0.get(index).copied() }
                pub fn iter(&self) -> impl Iterator<Item=usize> + '_ { self.0.iter().copied() }
            }
        }
        pub mod phys_map { pub fn phys_to_virt(pa: usize) -> Option<usize> { Some(pa) } }
        pub mod bitmap {
            use std::{alloc::{alloc, dealloc, Layout}, collections::BTreeSet, sync::{Mutex, LazyLock}};
            pub static LOCK: Mutex<()> = Mutex::new(());
            pub static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State {
                frames: BTreeSet::new(), remaining: None, events: Vec::new(),
            }));
            pub struct State { pub frames: BTreeSet<usize>, pub remaining: Option<usize>, pub events: Vec<usize> }
            pub fn allocate_frame() -> Option<usize> {
                let mut state = STATE.lock().unwrap();
                if let Some(left) = &mut state.remaining {
                    if *left == 0 { return None; }
                    *left -= 1;
                }
                let pointer = unsafe { alloc(Layout::from_size_align(4096,4096).unwrap()) };
                if pointer.is_null() { return None; }
                state.frames.insert(pointer as usize);
                Some(pointer as usize)
            }
            pub fn free_frame(pa: usize) -> Result<(), ()> {
                let mut state = STATE.lock().unwrap();
                assert!(state.frames.remove(&pa), "free of unowned table");
                crate::observe_charge_before_free();
                state.events.push(pa);
                unsafe { dealloc(pa as *mut u8, Layout::from_size_align(4096,4096).unwrap()); }
                Ok(())
            }
            pub fn snapshot() -> Vec<(usize, Vec<u64>)> {
                STATE.lock().unwrap().frames.iter().map(|&pa| (pa,
                    unsafe { std::slice::from_raw_parts(pa as *const u64, 512) }.to_vec())).collect()
            }
            pub fn fail_after(count: Option<usize>) { STATE.lock().unwrap().remaining = count; }
            pub fn count() -> usize { STATE.lock().unwrap().frames.len() }
        }
    }
}
fn paging_test_flush() { kernel::memory::bitmap::STATE.lock().unwrap().events.push(0); }
fn paging_test_publish(space: &Sv39AddressSpace, va: usize, count: usize) {
    assert_eq!(bitmap::STATE.lock().unwrap().events.last(), Some(&0), "publish before translation flush");
    for page in 0..count {
        assert!(space.query(va + page * PAGE).unwrap().1.executable, "publish before executable leaf");
    }
    bitmap::STATE.lock().unwrap().events.push(1);
}
use kernel::memory::{bitmap, vmo::VmoFrames};
use paging::{AddressSpace, MapFlags, PageError};
use sv39::Sv39AddressSpace;
const PAGE: usize = 4096;
unsafe fn slot(pa: usize, index: usize) -> *mut u64 { (pa as *mut u64).add(index) }
fn target(pte: u64) -> usize { ((pte >> 10) << 12) as usize }
unsafe fn leaf(space: &Sv39AddressSpace, va: usize) -> *mut u64 {
    let l1 = target(slot(space.root_paddr(), sv39::vpn2(va)).read());
    let l0 = target(slot(l1, sv39::vpn1(va)).read());
    slot(l0, sv39::vpn0(va))
}
#[test]
fn every_staging_failure_is_atomic_across_boundaries() {
    let _lock = bitmap::LOCK.lock().unwrap();
    for base in [0x1ff000, 0x3ffff000] {
        for fail in 0..4 {
            let mut space = Sv39AddressSpace::new_user().unwrap();
            let before = bitmap::snapshot();
            bitmap::fail_after(Some(fail));
            let frames = VmoFrames::contiguous(0x1000, 2);
            bitmap::STATE.lock().unwrap().events.clear();
            let result = unsafe { space.map_pages(base, &frames, 0, 2, MapFlags::user_rw()) };
            assert_eq!(bitmap::STATE.lock().unwrap().events.iter().filter(|&&event| event == 0).count(),
                usize::from(result.is_ok()));
            bitmap::fail_after(None);
            if result.is_err() {
                assert_eq!(result, Err(PageError::OutOfMemory));
                assert_eq!(bitmap::snapshot(), before);
                assert_eq!(space.table_pages(), 1);
            } else {
                assert_eq!(base, 0x1ff000);
                assert_eq!(fail, 3);
            }
        }
    }
    assert_eq!(bitmap::count(), 0);
}
#[test]
fn late_collisions_and_reserved_entries_are_atomic() {
    let _lock = bitmap::LOCK.lock().unwrap();
    let mut space = Sv39AddressSpace::new_user().unwrap();
    unsafe { space.map_page(0x200000, 0x9000, MapFlags::user_ro()).unwrap(); }
    let before = bitmap::snapshot();
    assert_eq!(unsafe { space.map_pages(0x1ff000, &VmoFrames::contiguous(0x1000,2), 0,2,MapFlags::user_rw()) }, Err(PageError::AlreadyMapped));
    assert_eq!(bitmap::snapshot(), before);
    unsafe {
        for raw in [1u64 << 63, sv39::PTE_V | sv39::PTE_R, sv39::PTE_V | sv39::PTE_U] {
            let root = slot(space.root_paddr(), 1);
            root.write(raw);
            let before = bitmap::snapshot();
            assert_eq!(space.map_page(1 << 30, 0x1000, MapFlags::user_rw()), Err(PageError::AlreadyMapped));
            assert_eq!(bitmap::snapshot(), before);
            root.write(0);
        }
        let p = leaf(&space, 0x200000);
        p.write(1 << 63);
        let before = bitmap::snapshot();
        assert_eq!(space.map_page(0x200000,0x1000,MapFlags::user_rw()),Err(PageError::AlreadyMapped));
        assert_eq!(bitmap::snapshot(),before);
    }
}
#[test]
fn exact_unmap_preflights_flags_frames_and_all_leaves() {
    let _lock = bitmap::LOCK.lock().unwrap();
    let mut space = Sv39AddressSpace::new_user().unwrap();
    let frames = VmoFrames::new(vec![0x1000,0x7000,0x3000]);
    unsafe { space.map_pages(0x1ff000,&frames,0,3,MapFlags::user_rw()).unwrap(); }
    let before = bitmap::snapshot();
    for flags in [MapFlags::user_ro(), MapFlags::user_rx()] {
        assert_eq!(unsafe { space.unmap_pages_exact(0x1ff000,&frames,0,3,flags) },Err(PageError::NotMapped));
        assert_eq!(bitmap::snapshot(),before);
    }
    let wrong = VmoFrames::new(vec![0x1000,0x7000,0x9000]);
    assert_eq!(unsafe { space.unmap_pages_exact(0x1ff000,&wrong,0,3,MapFlags::user_rw()) },Err(PageError::NotMapped));
    assert_eq!(bitmap::snapshot(),before);
    unsafe { space.unmap_pages_exact(0x1ff000,&frames,0,3,MapFlags::user_rw()).unwrap(); }
    assert_eq!(space.table_pages(),1);
}
#[test]
fn sparse_quota_reclaims_tables_and_flushes_before_free() {
    let _lock = bitmap::LOCK.lock().unwrap();
    let mut space = Sv39AddressSpace::new_user().unwrap();
    for i in 0..254 { unsafe { space.map_page(i << 21,0x1000,MapFlags::user_ro()).unwrap(); } }
    assert_eq!(space.table_pages(),256);
    assert_eq!(space.check_free_range(254 << 21,1),Ok(()));
    assert_eq!(space.check_free_range(0,0),Err(PageError::OutOfRange));
    unsafe { slot(space.root_paddr(),1).write(1 << 63); }
    assert_eq!(space.check_free_range(1 << 30,1),Err(PageError::AlreadyMapped));
    assert_eq!(unsafe { space.map_page(1 << 30,0x1000,MapFlags::user_ro()) },Err(PageError::AlreadyMapped));
    unsafe { slot(space.root_paddr(),1).write(0); }
    let before = bitmap::snapshot();
    assert_eq!(unsafe { space.map_page(254 << 21,0x1000,MapFlags::user_ro()) },Err(PageError::TableBudget));
    assert_eq!(bitmap::snapshot(),before);
    bitmap::STATE.lock().unwrap().events.clear();
    unsafe { space.unmap_page(0).unwrap(); }
    let events = bitmap::STATE.lock().unwrap().events.clone();
    assert_eq!(events.len(),2);
    assert_eq!(events[0],0);
    assert_ne!(events[1],0);
    assert!(space.query(1 << 21).is_some());
    unsafe { space.map_page(254 << 21,0x1000,MapFlags::user_ro()).unwrap(); }
    for i in 1..255 { unsafe { space.unmap_page(i << 21).unwrap(); } }
    assert_eq!(space.table_pages(),1);
    assert_eq!(bitmap::count(),1);
}
#[test]
fn reserved_sibling_prevents_pruning_and_validation_precedes_allocations() {
    let _lock = bitmap::LOCK.lock().unwrap();
    let mut space = Sv39AddressSpace::new_user().unwrap();
    unsafe {
        space.map_page(0,0x1000,MapFlags::user_rw()).unwrap();
        leaf(&space,0).add(1).write(1 << 63);
        space.unmap_page(0).unwrap();
    }
    assert_eq!(space.table_pages(),3);
    let before = bitmap::snapshot();
    for (va,frames,first,count,error) in [
        (1,VmoFrames::contiguous(0x1000,1),0,1,PageError::Unaligned),
        (0,VmoFrames::contiguous(0x1001,1),0,1,PageError::Unaligned),
        (0,VmoFrames::contiguous(1 << 56,1),0,1,PageError::OutOfRange),
        (0,VmoFrames::contiguous(0x1000,1),1,1,PageError::OutOfRange),
        ((1 << 38)-PAGE,VmoFrames::contiguous(0x1000,2),0,2,PageError::OutOfRange),
        (0xffff_ffc0_0000_0000,VmoFrames::contiguous(0x1000,1),0,1,PageError::OutOfRange),
    ] {
        assert_eq!(unsafe { space.map_pages(va,&frames,first,count,MapFlags::user_rw()) },Err(error));
        assert_eq!(bitmap::snapshot(),before);
    }
}

#[test]
fn metadata_oom_and_late_physical_oom_keep_existing_prefixes_intact() {
    let _lock = bitmap::LOCK.lock().unwrap();
    let mut space = Sv39AddressSpace::new_user().unwrap();
    unsafe { space.map_page(0,0x9000,MapFlags::user_ro()).unwrap(); }
    let frames = VmoFrames::contiguous(0x1000,513);
    let before = bitmap::snapshot();
    FAIL_METADATA.with(|flag| flag.set(true));
    let result = unsafe { space.map_pages(PAGE,&frames,0,513,MapFlags::user_rw()) };
    FAIL_METADATA.with(|flag| flag.set(false));
    assert_eq!(result,Err(PageError::OutOfMemory));
    assert_eq!(bitmap::snapshot(),before);
    bitmap::fail_after(Some(0));
    let result = unsafe { space.map_pages(PAGE,&frames,0,513,MapFlags::user_rw()) };
    bitmap::fail_after(None);
    assert_eq!(result,Err(PageError::OutOfMemory));
    assert_eq!(bitmap::snapshot(),before);
    unsafe { space.map_pages(PAGE,&frames,0,513,MapFlags::user_rw()).unwrap(); }
    assert_eq!(space.table_pages(),4);
}

#[test]
fn executable_publication_follows_commit_and_flush_only_on_success() {
    let _lock = bitmap::LOCK.lock().unwrap();
    let frames = VmoFrames::contiguous(0x1000, 2);
    for flags in [MapFlags::user_ro(), MapFlags::user_rw(), MapFlags::user_rx()] {
        for fail in 0..=4 {
            let mut space = Sv39AddressSpace::new_user().unwrap();
            bitmap::fail_after(Some(fail));
            bitmap::STATE.lock().unwrap().events.clear();
            let result = unsafe { space.map_pages(0x3ffff000, &frames, 0, 2, flags) };
            bitmap::fail_after(None);
            let events = bitmap::STATE.lock().unwrap().events.clone();
            assert_eq!(events.iter().filter(|&&event| event == 1).count(),
                usize::from(result.is_ok() && flags.executable));
            if result.is_ok() {
                assert_eq!(events, if flags.executable { vec![0, 1] } else { vec![0] });
                assert_eq!(space.query(0x3ffff000), Some((0x1000, flags)));
                assert_eq!(space.query(0x40000000), Some((0x2000, flags)));
            } else {
                assert_eq!(result, Err(PageError::OutOfMemory));
                assert!(space.query(0x3ffff000).is_none());
            }
        }
    }
    assert_eq!(bitmap::count(), 0);
}

#[test]
fn retained_owner_ancestor_quota_and_free_before_decharge() {
    use accounting::{BudgetRef, JobBudget, with_budget};
    let _lock = bitmap::LOCK.lock().unwrap();
    let system_before = BudgetRef::system().used();
    let parent = JobBudget::child(&BudgetRef::system(), 4 * PAGE).unwrap();
    let owner = JobBudget::child(&parent, 8 * PAGE).unwrap();
    let mut space = with_budget(&owner, Sv39AddressSpace::new_user).unwrap();
    assert!(space.budget().same_owner(&owner));
    assert_eq!(owner.used(), PAGE);
    unsafe { space.map_page(0, 0x1000, MapFlags::user_rw()).unwrap(); }
    assert_eq!(owner.used(), 3 * PAGE);
    assert_eq!(parent.used(), owner.used());
    CHARGE_OBSERVER.with(|observer| *observer.borrow_mut() = Some(owner.clone()));
    let before = bitmap::snapshot();
    assert_eq!(unsafe { space.map_page(1 << 30, 0x2000, MapFlags::user_ro()) }, Err(PageError::JobQuota));
    assert_eq!(bitmap::snapshot(), before, "ancestor rejection published partial tables");
    assert_eq!(owner.used(), 3 * PAGE);
    assert_eq!(*FREE_CHARGES.lock().unwrap(), [4 * PAGE], "pending frame decharged before raw free");
    FREE_CHARGES.lock().unwrap().clear();
    owner.close();
    assert_eq!(unsafe { space.map_page(PAGE, 0x2000, MapFlags::user_ro()) }, Err(PageError::JobQuota));
    unsafe { space.unmap_page(0).unwrap(); }
    assert_eq!(owner.used(), PAGE);
    drop(space);
    assert_eq!(*FREE_CHARGES.lock().unwrap(), [3 * PAGE, 2 * PAGE, PAGE]);
    CHARGE_OBSERVER.with(|observer| *observer.borrow_mut() = None);
    FREE_CHARGES.lock().unwrap().clear();
    assert_eq!(owner.used(), 0);
    assert_eq!(parent.used(), 0);
    drop(owner);
    drop(parent);
    assert_eq!(BudgetRef::system().used(), system_before);
    assert_eq!(bitmap::count(), 0);
}

#[test]
fn device_mapping_is_nx_and_transactional() {
    let _lock = bitmap::LOCK.lock().unwrap();
    let mut space = Sv39AddressSpace::new_user().unwrap();
    let before = bitmap::snapshot();
    assert_eq!(unsafe { space.map_device_pages(0, 0x10000000, 1, MapFlags::user_rx()) }, Err(PageError::OutOfRange));
    assert_eq!(bitmap::snapshot(), before);
    for fail in 0..2 {
        bitmap::fail_after(Some(fail));
        assert_eq!(unsafe { space.map_device_pages(0, 0x10000000, 1, MapFlags::user_rw()) }, Err(PageError::OutOfMemory));
        bitmap::fail_after(None);
        assert_eq!(bitmap::snapshot(), before);
    }
    unsafe { space.map_device_pages(0, 0x10000000, 1, MapFlags::user_rw()).unwrap(); }
    assert_eq!(space.query(0), Some((0x10000000, MapFlags::user_rw())));
    assert_eq!(unsafe { leaf(&space, 0).read() } & (1 << 3), 0);
    unsafe { space.unmap_page(0).unwrap(); }
}
