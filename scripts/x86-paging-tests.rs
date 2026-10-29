//! Run the production x86 walker against an instrumented frame allocator.
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


#[path = "../ferber/src/boot_info.rs"] mod boot_info;
#[path = "../ferber/src/kernel/memory/paging.rs"] pub mod paging_hal;
#[path = "../ferber/src/arch/x86_64/paging.rs"] mod paging;
mod arch { pub use crate::paging::Pml4AddressSpace as ArchAddressSpace; }
mod kernel { pub use crate::sync; pub mod memory {
    pub use crate::accounting;
    pub use crate::paging_hal as paging;
    pub use crate::mock_bitmap as bitmap;
    pub use crate::mock_phys as phys_map;
    pub mod vmo {
        pub struct VmoFrames { base: usize, count: usize }
        impl VmoFrames {
            pub fn contiguous(base: usize, count: usize) -> Self { Self { base, count } }
            pub fn count(&self) -> usize { self.count }
            pub fn iter(&self) -> impl Iterator<Item=usize> + '_ { (0..self.count).map(|i| self.get(i).unwrap()) }
            pub fn get(&self, i: usize) -> Option<usize> { if i < self.count { self.base.checked_add(i.checked_mul(4096)?) } else { None } }
        }
    }
}}
pub mod mock_phys {
    pub struct Extent { pub start: usize, pub end: usize, pub virtual_start: usize }
    pub fn phys_to_virt(p: usize) -> Option<usize> { Some(p) }
    pub fn extents() -> impl Iterator<Item = Extent> { core::iter::empty() }
    pub fn bootstrap_allocate_page() -> Option<usize> { None }
    pub fn activate() {}
}
pub mod mock_bitmap {
    use std::sync::Mutex;
    pub static EVENTS: Mutex<Vec<bool>> = Mutex::new(Vec::new());
    pub static STATE: Mutex<(Vec<usize>, usize, usize)> = Mutex::new((Vec::new(), usize::MAX, 0));
    pub fn allocate_frame() -> Option<usize> {
        let mut state = STATE.lock().unwrap();
        if state.1 == 0 { return None; }
        state.1 -= 1;
        let pa = unsafe { std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(4096,4096).unwrap()) } as usize;
        assert_ne!(pa, 0); state.0.push(pa); Some(pa)
    }
    pub fn free_frame(pa: usize) -> Result<(), ()> {
        crate::observe_charge_before_free();
        EVENTS.lock().unwrap().push(false);
        let mut state = STATE.lock().unwrap();
        let i = state.0.iter().position(|p| *p == pa).unwrap(); state.0.remove(i);
        unsafe { std::alloc::dealloc(pa as *mut u8, std::alloc::Layout::from_size_align(4096,4096).unwrap()); }
        Ok(())
    }
    pub struct Stats { pub free_pages: usize }
    pub fn stats() -> Stats { Stats { free_pages: usize::MAX - STATE.lock().unwrap().0.len() } }
}
pub fn test_flush(full: bool) { if full { mock_bitmap::EVENTS.lock().unwrap().push(true); mock_bitmap::STATE.lock().unwrap().2 += 1; } }
pub static PUBLICATIONS: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());
pub fn paging_test_publish(space: &paging::Pml4AddressSpace, va: usize, count: usize) {
    use paging_hal::AddressSpace;
    assert_eq!(mock_bitmap::EVENTS.lock().unwrap().last(), Some(&true), "publish before translation flush");
    for page in 0..count {
        assert!(space.query(va + page * 4096).unwrap().1.executable, "publish before executable leaf");
    }
    PUBLICATIONS.lock().unwrap().push((va, count));
}
