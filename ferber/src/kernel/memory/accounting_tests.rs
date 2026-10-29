//! Native checks use the actual global allocator, including Arc storage.
use super::{accounting::{self, AllocError, BudgetRef, FrameCharge, JobBudget}, bitmap, Vmo};
use alloc::alloc::{alloc, dealloc, realloc, Layout};
use alloc::sync::Arc;
const PAGE: usize = 4096;

unsafe fn allocate(layout: Layout) -> Result<*mut u8, AllocError> {
    accounting::attempt(|| {
        let pointer = unsafe { alloc(layout) };
        if pointer.is_null() { Err(()) } else { Ok(pointer) }
    })
}

// 2026-10-27 09:04 AST: edesur killed the parent Job mid-accounting.
// all children went dark. the bill still has a live owner.
// apparently that reference never drops.
fn batch() {
    let before = bitmap::stats().free_pages;
    let charged = BudgetRef::system().used();
    {
        let a = JobBudget::child(&BudgetRef::system(), 2 * PAGE).unwrap();
        let b = JobBudget::child(&BudgetRef::system(), 2 * PAGE).unwrap();
        let layout = Layout::from_size_align(8, 8).unwrap();
        let first = accounting::with_budget(&a, || unsafe { allocate(layout).unwrap() });
        let second = accounting::with_budget(&b, || unsafe { allocate(layout).unwrap() });
        assert_eq!(a.used(), PAGE, "Job arena must own its complete page");
        assert_eq!(b.used(), PAGE);
        unsafe { first.write(0x74); second.write(0x19); }
        let grown = accounting::with_budget(&b, || accounting::attempt(|| {
            let pointer = unsafe { realloc(first, layout, 8192) };
            if pointer.is_null() { Err(()) } else { Ok(pointer) }
        }));
        assert_eq!(grown, Err(AllocError::Quota), "realloc must preserve original owner");
        assert_eq!(unsafe { first.read() }, 0x74);
        a.close();
        assert!(matches!(accounting::with_budget(&a, || accounting::attempt(|| Arc::try_new(7u64))), Err(AllocError::Quota)));
        accounting::with_budget(&b, || unsafe { dealloc(first, layout); dealloc(second, layout); });
        assert_eq!(a.used(), 0);
        assert_eq!(b.used(), 0);
    }
    {
        let parent = JobBudget::child(&BudgetRef::system(), 8 * PAGE).unwrap();
        let a = JobBudget::child(&parent, 16 * PAGE).unwrap();
        let b = JobBudget::child(&parent, 16 * PAGE).unwrap();
        let memory = accounting::with_budget(&a, || {
            let memory = Vmo::create(PAGE).unwrap();
            accounting::attempt(|| Arc::try_new(memory)).unwrap()
        });
        let other = accounting::with_budget(&b, || Vmo::create(PAGE).unwrap());
        assert!(a.used() >= 2 * PAGE, "backing and metadata are distinct owned pages");
        let usage = parent.used();
        let pages = (parent.limit() - usage) / PAGE + 1;
        assert!(a.used() + pages * PAGE <= a.limit());
        assert!(matches!(FrameCharge::reserve(&a, pages), Err(AllocError::Quota)));
        assert_eq!(parent.used(), usage, "ancestor rejection changed accounting");
        a.close();
        let retained = memory.clone();
        drop(memory);
        let creator_charge = a.used();
        accounting::with_budget(&b, || {
            retained.write_owned(0, &[0x74]).unwrap();
            let mut byte = [0];
            retained.read_owned(0, &mut byte).unwrap();
            assert_eq!(byte, [0x74]);
        });
        assert_eq!(a.used(), creator_charge, "foreign use moved backing charge");
        accounting::with_budget(&b, || drop(retained));
        assert_eq!(a.used(), 0, "foreign final owner retained backing charge");
        drop(other);
        assert_eq!(b.used(), 0);
        parent.close();
        assert!(matches!(accounting::with_budget(&b, || Vmo::create(PAGE)), Err(super::VmoError::JobQuota)));
    }
    assert_eq!(bitmap::stats().free_pages, before, "native Job accounting frame leak");
    assert_eq!(BudgetRef::system().used(), charged, "native Job accounting charge leak");
}

pub fn run() {
    accounting::assert_switch_safe();
    assert!(accounting::current().same_owner(&BudgetRef::system()));
    batch();
    batch();
    reconcile();
    crate::kprintln!("[mem]   hierarchical Job accounting and allocation ownership: ok");
}

pub fn reconcile() {
    let stats = bitmap::stats();
    assert_eq!(BudgetRef::system().used(),
        (stats.bootstrap_reserved_pages + stats.allocated_pages) * PAGE,
        "managed frame ownership differs from Job accounting");
}
