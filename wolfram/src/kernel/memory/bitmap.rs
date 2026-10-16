//! Physical frames across discovered RAM, with independently carved metadata pages.

use super::phys_map;
use crate::boot_info::BootInfo;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

const PAGE_SIZE: usize = 4096;
const WORDS: usize = 254;
const FRAMES: usize = WORDS * 64;

#[derive(Clone, Copy)]
pub struct MemoryStats {
    pub start: usize,
    pub end: usize,
    pub total_pages: usize,
    pub free_pages: usize,
    pub bootstrap_reserved_pages: usize,
    pub allocated_pages: usize,
    pub extent_count: usize,
}

#[repr(C)]
struct Block {
    next: usize,
    start: usize,
    frames: usize,
    used: [u64; WORDS],
    permanent: [u64; WORDS],
}
const _: () = assert!(core::mem::size_of::<Block>() <= PAGE_SIZE);

struct State {
    head: usize,
    hint: usize,
    hint_word: usize,
    contiguous_hint: usize,
    contiguous_count: usize,
    stats: MemoryStats,
    initialized: bool,
}
struct LockedState {
    lock: AtomicBool,
    state: UnsafeCell<State>,
}
// The lock serializes all allocation state and metadata writes.
unsafe impl Sync for LockedState {}
static ALLOCATOR: LockedState = LockedState {
    lock: AtomicBool::new(false),
    state: UnsafeCell::new(State {
        head: 0,
        hint: 0,
        hint_word: 0,
        contiguous_hint: 0,
        contiguous_count: 0,
        stats: MemoryStats {
            start: 0,
            end: 0,
            total_pages: 0,
            free_pages: 0,
            bootstrap_reserved_pages: 0,
            allocated_pages: 0,
            extent_count: 0,
        },
        initialized: false,
    }),
};
impl LockedState {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        struct Unlock<'a>(&'a AtomicBool);
        impl Drop for Unlock<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _unlock = Unlock(&self.lock);
        f(unsafe { &mut *self.state.get() })
    }
}

// Metadata physical addresses remain stable across the permanent-map switch.
unsafe fn block(address: usize) -> &'static mut Block {
    &mut *(phys_map::phys_to_virt(address).expect("bitmap metadata alias") as *mut Block)
}

impl State {
    fn locate(&self, address: usize) -> Option<(usize, usize)> {
        let mut current = self.head;
        while current != 0 {
            let b = unsafe { block(current) };
            if address >= b.start && address < b.start + b.frames * PAGE_SIZE {
                return Some((current, (address - b.start) / PAGE_SIZE));
            }
            current = b.next;
        }
        None
    }
    fn mark(&mut self, address: usize, permanent: bool) {
        let (pa, index) = self.locate(address).expect("frame metadata missing");
        let b = unsafe { block(pa) };
        let bit = 1u64 << (index % 64);
        if b.used[index / 64] & bit == 0 {
            b.used[index / 64] |= bit;
            self.stats.free_pages -= 1;
            if !permanent {
                self.stats.allocated_pages += 1;
            }
        }
        if permanent {
            b.permanent[index / 64] |= bit;
        }
    }
}

pub fn init_from_boot_info(info: &BootInfo) -> MemoryStats {
    info.validate()
        .unwrap_or_else(|e| panic!("physical memory detection: {}", e));
    let extents = phys_map::extents();
    ALLOCATOR.with(|state| {
        assert!(!state.initialized, "physical allocator initialized twice");
        state.stats.extent_count = extents.len();
        state.stats.start = extents[0].start;
        state.stats.end = extents.last().unwrap().end;
        let mut metadata_cursor = extents[0].start;
        for (i, _) in extents.iter().enumerate() {
            state.stats.bootstrap_reserved_pages += phys_map::bootstrap_reserved_pages(i);
        }
        let mut previous = 0;
        for extent in extents {
            let mut start = extent.start;
            while start < extent.end {
                // Metadata pages can be fragmented and need no heap allocation.
                let metadata = {
                    let candidate = extents
                        .iter()
                        .enumerate()
                        .find_map(|(i, e)| {
                            let p = metadata_cursor
                                .max(e.start + phys_map::bootstrap_reserved_pages(i) * PAGE_SIZE);
                            (p < e.end).then_some(p)
                        })
                        .expect("no bootstrap-accessible RAM for frame metadata");
                    metadata_cursor = candidate
                        .checked_add(PAGE_SIZE)
                        .expect("metadata address overflow");
                    candidate
                };
                let frames = ((extent.end - start) / PAGE_SIZE).min(FRAMES);
                let b = unsafe { block(metadata) };
                b.next = 0;
                b.start = start;
                b.frames = frames;
                b.used.fill(u64::MAX);
                b.permanent.fill(u64::MAX);
                for index in 0..frames {
                    let bit = 1u64 << (index % 64);
                    b.used[index / 64] &= !bit;
                    b.permanent[index / 64] &= !bit;
                }
                state.stats.free_pages += frames;
                state.stats.bootstrap_reserved_pages += 1;
                if previous == 0 {
                    state.head = metadata;
                } else {
                    unsafe { block(previous) }.next = metadata;
                }
                previous = metadata;
                state.stats.total_pages += frames;
                start += frames * PAGE_SIZE;
            }
        }
        for (i, extent) in extents.iter().enumerate() {
            for page in 0..phys_map::bootstrap_reserved_pages(i) {
                state.mark(extent.start + page * PAGE_SIZE, true);
            }
        }
        let mut current = state.head;
        while current != 0 {
            state.mark(current, true);
            current = unsafe { block(current) }.next;
        }
        state.hint = state.head;
        state.initialized = true;
        state.stats
    })
}

pub fn stats() -> MemoryStats {
    ALLOCATOR.with(|state| {
        assert!(state.initialized, "physical allocator used before init");
        state.stats
    })
}

pub fn allocate_frame() -> Option<usize> {
    ALLOCATOR.with(|state| {
        assert!(state.initialized, "physical allocator used before init");
        let mut current = state.hint;
        while current != 0 {
            let b = unsafe { block(current) };
            for word in state.hint_word..b.frames.div_ceil(64) {
                let available = !b.used[word];
                if available == 0 {
                    continue;
                }
                let index = word * 64 + available.trailing_zeros() as usize;
                let address = b.start + index * PAGE_SIZE;
                b.used[word] |= 1 << (index % 64);
                state.stats.free_pages -= 1;
                state.stats.allocated_pages += 1;
                state.hint = current;
                state.hint_word = word;
                return Some(address);
            }
            current = b.next;
            state.hint_word = 0;
        }
        state.hint = 0;
        None
    })
}

/// Target a physical range without consuming every lower frame first.
pub fn allocate_frame_above(minimum: usize) -> Option<usize> {
    ALLOCATOR.with(|state| {
        assert!(state.initialized, "physical allocator used before init");
        let mut current = state.head;
        while current != 0 {
            let b = unsafe { block(current) };
            let first = minimum
                .saturating_sub(b.start)
                .div_ceil(PAGE_SIZE)
                .min(b.frames);
            for index in first..b.frames {
                if b.used[index / 64] & (1 << (index % 64)) == 0 {
                    let address = b.start + index * PAGE_SIZE;
                    state.mark(address, false);
                    return Some(address);
                }
            }
            current = b.next;
        }
        None
    })
}

pub fn allocate_frames(count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if count == 1 {
        return allocate_frame();
    }
    ALLOCATOR.with(|state| {
        assert!(state.initialized, "physical allocator used before init");
        if state.stats.free_pages < count {
            return None;
        }
        let mut current = state.head;
        let mut run = 0;
        let mut start = 0;
        let mut expected = 0;
        let skip = if count >= state.contiguous_count {
            state.contiguous_hint
        } else {
            0
        };
        while current != 0 {
            let b = unsafe { block(current) };
            let first = skip
                .saturating_sub(b.start)
                .div_ceil(PAGE_SIZE)
                .min(b.frames);
            for index in first..b.frames {
                let address = b.start + index * PAGE_SIZE;
                if address != expected || b.used[index / 64] & (1 << (index % 64)) != 0 {
                    run = 0;
                }
                if b.used[index / 64] & (1 << (index % 64)) == 0 {
                    if run == 0 {
                        start = address;
                    }
                    run += 1;
                    if run == count {
                        for i in 0..count {
                            state.mark(start + i * PAGE_SIZE, false);
                        }
                        state.contiguous_hint = start + count * PAGE_SIZE;
                        state.contiguous_count = count;
                        return Some(start);
                    }
                }
                expected = address + PAGE_SIZE;
            }
            current = b.next;
        }
        None
    })
}

pub fn free_frame(address: usize) -> Result<(), &'static str> {
    free_frames(address, 1)
}

pub fn free_frames(address: usize, count: usize) -> Result<(), &'static str> {
    ALLOCATOR.with(|state| {
        if !state.initialized {
            return Err("physical allocator used before init");
        }
        if count == 0
            || !address.is_multiple_of(PAGE_SIZE)
            || count
                .checked_mul(PAGE_SIZE)
                .and_then(|bytes| address.checked_add(bytes))
                .is_none()
        {
            return Err("invalid frame range");
        }
        // Validate the complete operation before releasing any frame.
        for i in 0..count {
            let (pa, index) = state
                .locate(address + i * PAGE_SIZE)
                .ok_or("frame outside managed RAM")?;
            let b = unsafe { block(pa) };
            let bit = 1u64 << (index % 64);
            if b.permanent[index / 64] & bit != 0 {
                return Err("frame is reserved");
            }
            if b.used[index / 64] & bit == 0 {
                return Err("frame is already free");
            }
        }
        for i in 0..count {
            let (pa, index) = state.locate(address + i * PAGE_SIZE).unwrap();
            unsafe { block(pa) }.used[index / 64] &= !(1 << (index % 64));
        }
        state.stats.free_pages += count;
        state.stats.allocated_pages -= count;
        state.hint = state.head;
        state.hint_word = 0;
        state.contiguous_hint = 0;
        state.contiguous_count = 0;
        Ok(())
    })
}

pub fn self_test() {
    let before = stats().free_pages;
    let first = allocate_frames(2).expect("frame test allocation");
    free_frame(first + PAGE_SIZE).unwrap();
    assert!(free_frames(first, 2).is_err());
    assert_eq!(
        stats().free_pages,
        before - 1,
        "failed free partially released frames"
    );
    assert!(free_frames(usize::MAX - PAGE_SIZE + 1, 2).is_err());
    free_frame(first).unwrap();
    assert_eq!(stats().free_pages, before);
    for extent in phys_map::extents() {
        assert_eq!(
            phys_map::virt_to_phys(phys_map::phys_to_virt(extent.start).unwrap()),
            Some(extent.start)
        );
        assert!(extent.start < extent.end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_info::MemoryRegion;

    #[test]
    fn sparse_allocator_crosses_metadata_blocks_and_rejects_partial_frees() {
        let bytes = 192 * 1024 * 1024;
        let layout = std::alloc::Layout::from_size_align(bytes, PAGE_SIZE).unwrap();
        let ram = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!ram.is_null());
        let start = ram as usize;
        let hole = start + 144 * 1024 * 1024;
        let regions = [
            MemoryRegion::usable(start as u64, (start + bytes) as u64),
            MemoryRegion::usable((start + PAGE_SIZE) as u64, (start + bytes / 2) as u64),
            MemoryRegion::reserved((hole + 1) as u64, (hole + PAGE_SIZE - 1) as u64),
        ];
        let info = BootInfo::new(
            &regions,
            PAGE_SIZE as u64,
            (PAGE_SIZE * 2) as u64,
            0xffff_ffff_8000_0000,
            0x10000,
            regions.as_ptr() as u64,
        );
        phys_map::init(&info);
        let bootstrap = phys_map::bootstrap_allocate_page().unwrap();
        let initial = init_from_boot_info(&info);
        assert_eq!(initial.total_pages, bytes / PAGE_SIZE - 1);
        assert_eq!(initial.extent_count, 2);
        assert_eq!(initial.allocated_pages, 0);
        assert_eq!(
            initial.total_pages,
            initial.bootstrap_reserved_pages + initial.free_pages
        );
        assert!(free_frame(bootstrap).is_err());
        self_test();
        let large = allocate_frames(FRAMES + 1).expect("allocation must cross metadata blocks");
        assert!(large + (FRAMES + 1) * PAGE_SIZE <= hole);
        free_frames(large, FRAMES + 1).unwrap();
        let mut allocated = std::vec::Vec::new();
        while let Some(frame) = allocate_frame() {
            assert_ne!(frame, hole);
            allocated.push(frame);
        }
        assert_eq!(allocated.len(), initial.free_pages);
        assert_eq!(stats().free_pages, 0);
        assert!(allocate_frames(1).is_none());
        assert!(allocate_frame_above(start + bytes).is_none());
        for frame in allocated {
            free_frame(frame).unwrap();
        }
        assert_eq!(stats().free_pages, initial.free_pages);
        assert_eq!(stats().allocated_pages, 0);
        unsafe { std::alloc::dealloc(ram, layout) };
    }
}
