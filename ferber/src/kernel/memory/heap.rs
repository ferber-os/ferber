//! Per-owner buddy arenas. Every backing page is charged once, including slack.
use super::{
    accounting::{self, AllocError, BudgetRef, JobBudget},
    bitmap, phys_map,
};
use crate::kernel::sync::SpinLock;
use core::alloc::{GlobalAlloc, Layout};
use core::{mem, ptr};

const PAGE_SIZE: usize = 4096;
const MIN_BLOCK_SIZE: usize = 32;
const MAX_ORDER: usize = 7;
pub const MAX_ALLOC_SIZE: usize = 128 * 1024 * 1024;

pub(super) struct Arena {
    free: [*mut u8; MAX_ORDER + 1],
}
impl Arena {
    pub const fn new() -> Self {
        Self {
            free: [ptr::null_mut(); MAX_ORDER + 1],
        }
    }
    pub fn is_empty(&self) -> bool {
        self.free.iter().all(|p| p.is_null())
    }
}
#[repr(C)]
struct Header {
    owner: *const JobBudget,
    base: *mut u8,
    span: usize,
    large: usize,
}
pub const HEADER_SIZE: usize = mem::size_of::<Header>();
static HEAP_LOCK: SpinLock<()> = SpinLock::new(());
pub struct BuddyAllocator;
#[cfg_attr(not(test), global_allocator)]
static HEAP_ALLOCATOR: BuddyAllocator = BuddyAllocator;

fn extent(layout: Layout) -> Option<(usize, usize)> {
    if layout.size() == 0
        || layout.size().max(layout.align()) > MAX_ALLOC_SIZE
        || layout.align() > PAGE_SIZE
    {
        return None;
    }
    let align = layout.align().max(mem::align_of::<Header>());
    let offset = HEADER_SIZE.checked_add(align - 1)? & !(align - 1);
    Some((offset, offset.checked_add(layout.size())?))
}
fn order(size: usize) -> usize {
    size.next_power_of_two()
        .max(MIN_BLOCK_SIZE)
        .trailing_zeros() as usize
        - 5
}
unsafe fn pop(arena: &mut Arena, index: usize) -> *mut u8 {
    let block = arena.free[index];
    arena.free[index] = unsafe { block.cast::<*mut u8>().read() };
    block
}
unsafe fn push(arena: &mut Arena, index: usize, block: *mut u8) {
    unsafe {
        block.cast::<*mut u8>().write(arena.free[index]);
    }
    arena.free[index] = block;
}
impl BuddyAllocator {
    unsafe fn allocate(&self, layout: Layout, owner: &BudgetRef) -> *mut u8 {
        let Some((offset, required)) = extent(layout) else {
            accounting::failed(AllocError::OutOfMemory);
            return ptr::null_mut();
        };
        let result = {
            let _heap = HEAP_LOCK.lock();
            accounting::transaction(|ledger| {
                owner.open_locked(ledger)?;
                let (base, span, large) = if required > PAGE_SIZE {
                    let pages = required.div_ceil(PAGE_SIZE);
                    let span = pages * PAGE_SIZE;
                    owner.reserve_locked(span, ledger)?;
                    let Some(physical) = bitmap::allocate_frames(pages) else {
                        owner.release_locked(span, ledger);
                        return Err(AllocError::OutOfMemory);
                    };
                    (
                        phys_map::phys_to_virt(physical).expect("heap RAM alias") as *mut u8,
                        span,
                        1,
                    )
                } else {
                    let wanted = order(required);
                    let arena = unsafe { &mut *owner.budget().arena.get() };
                    let available =
                        (wanted..=MAX_ORDER).find(|index| !arena.free[*index].is_null());
                    let (block, mut level) = if let Some(level) = available {
                        (unsafe { pop(arena, level) }, level)
                    } else {
                        owner.reserve_locked(PAGE_SIZE, ledger)?;
                        let Some(physical) = bitmap::allocate_frame() else {
                            owner.release_locked(PAGE_SIZE, ledger);
                            return Err(AllocError::OutOfMemory);
                        };
                        (
                            phys_map::phys_to_virt(physical).expect("heap RAM alias") as *mut u8,
                            MAX_ORDER,
                        )
                    };
                    while level > wanted {
                        level -= 1;
                        unsafe {
                            push(arena, level, block.add(MIN_BLOCK_SIZE << level));
                        }
                    }
                    (block, MIN_BLOCK_SIZE << wanted, 0)
                };
                let user = unsafe { base.add(offset) };
                unsafe {
                    user.sub(HEADER_SIZE).cast::<Header>().write(Header {
                        owner: owner.retain_raw(),
                        base,
                        span,
                        large,
                    });
                }
                Ok(user)
            })
        };
        match result {
            Ok(pointer) => pointer,
            Err(error) => {
                accounting::failed(error);
                ptr::null_mut()
            }
        }
    }
}
unsafe impl GlobalAlloc for BuddyAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let owner = accounting::current();
        unsafe { self.allocate(layout, &owner) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, _: Layout) {
        if pointer.is_null() {
            return;
        }
        let header = unsafe { pointer.sub(HEADER_SIZE).cast::<Header>().read() };
        let owner = unsafe { BudgetRef::from_raw(header.owner) };
        {
            let _heap = HEAP_LOCK.lock();
            accounting::transaction(|ledger| {
                if header.large != 0 {
                    bitmap::free_frames(
                        phys_map::virt_to_phys(header.base as usize)
                            .expect("heap physical address"),
                        header.span / PAGE_SIZE,
                    )
                    .expect("invalid heap span free");
                    owner.release_locked(header.span, ledger);
                } else {
                    let arena = unsafe { &mut *owner.budget().arena.get() };
                    let mut block = header.base as usize;
                    let mut level = order(header.span);
                    while level < MAX_ORDER {
                        let buddy = (block ^ (MIN_BLOCK_SIZE << level)) as *mut u8;
                        let mut link = &mut arena.free[level] as *mut *mut u8;
                        while unsafe { !(*link).is_null() && *link != buddy } {
                            link = unsafe { (*link).cast::<*mut u8>() };
                        }
                        if unsafe { (*link).is_null() } {
                            break;
                        }
                        unsafe {
                            *link = buddy.cast::<*mut u8>().read();
                        }
                        block = block.min(buddy as usize);
                        level += 1;
                    }
                    if level == MAX_ORDER {
                        bitmap::free_frame(
                            phys_map::virt_to_phys(block).expect("heap physical address"),
                        )
                        .expect("invalid heap page free");
                        owner.release_locked(PAGE_SIZE, ledger);
                    } else {
                        unsafe {
                            push(arena, level, block as *mut u8);
                        }
                    }
                }
            });
        }
        // The last budget reference may free parent-owned ledger storage recursively.
        drop(owner);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let Ok(next_layout) = Layout::from_size_align(new_size, layout.align()) else {
            accounting::failed(AllocError::OutOfMemory);
            return ptr::null_mut();
        };
        if extent(next_layout).is_none() {
            accounting::failed(AllocError::OutOfMemory);
            return ptr::null_mut();
        }
        let header = unsafe { &*pointer.sub(HEADER_SIZE).cast::<Header>() };
        let offset = pointer as usize - header.base as usize;
        if new_size != 0 && new_size <= header.span - offset {
            return pointer;
        }
        let owner = unsafe { BudgetRef::clone_raw(header.owner) };
        let replacement = unsafe { self.allocate(next_layout, &owner) };
        if !replacement.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(pointer, replacement, layout.size().min(new_size));
                self.dealloc(pointer, layout);
            }
        }
        replacement
    }
}
