//! Packed supervisor aliases for discovered RAM; gaps consume no virtual space.

use crate::boot_info::{BootInfo, MEMORY_RESERVED, MEMORY_USABLE};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

const MAX_EXTENTS: usize = 8194;
const PAGE: usize = 4096;
#[cfg(target_arch = "riscv64")]
const PHYSICAL_END: u64 = 1 << 56;
#[cfg(target_arch = "x86_64")]
const PHYSICAL_END: u64 = 1 << 52;
#[cfg(target_arch = "riscv64")]
pub const ALIAS_START: usize = 0xffff_ffc0_0000_0000;
#[cfg(target_arch = "x86_64")]
pub const ALIAS_START: usize = 0xffff_8000_0000_0000;
pub const ALIAS_END: usize = 0xffff_fffe_0000_0000;

#[derive(Clone, Copy, Debug)]
pub struct Extent {
    pub start: usize,
    pub end: usize,
    pub virtual_start: usize,
}
const EMPTY: Extent = Extent {
    start: 0,
    end: 0,
    virtual_start: 0,
};
struct Map {
    entries: [Extent; MAX_EXTENTS],
    count: usize,
    kernel_start: usize,
    kernel_end: usize,
    kernel_virtual: usize,
    device: Extent,
}
struct Storage(UnsafeCell<Map>);
// Written once by the boot CPU before allocation and read immutably afterwards.
unsafe impl Sync for Storage {}
static MAP: Storage = Storage(UnsafeCell::new(Map {
    entries: [EMPTY; MAX_EXTENTS],
    count: 0,
    kernel_start: 0,
    kernel_end: 0,
    kernel_virtual: 0,
    device: EMPTY,
}));
struct Bootstrap(UnsafeCell<[usize; MAX_EXTENTS]>);
// Only the boot CPU carves pages, before the general allocator is initialized.
unsafe impl Sync for Bootstrap {}
static BOOTSTRAP: Bootstrap = Bootstrap(UnsafeCell::new([0; MAX_EXTENTS]));
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn init(info: &BootInfo) {
    // SAFETY: boot is single-threaded and invokes initialization once.
    let map = unsafe { &mut *MAP.0.get() };
    normalize(map, info);
}

fn normalize(map: &mut Map, info: &BootInfo) {
    assert_eq!(map.count, 0, "RAM aliases initialized twice");
    info.validate().expect("invalid boot information");
    map.kernel_start = info.kernel_start as usize;
    map.kernel_end = info.kernel_end as usize;
    map.kernel_virtual = info.kernel_virtual_start as usize;
    if info.framebuffer.size != 0 {
        let start = info.framebuffer.base as usize & !(PAGE - 1);
        let end = info
            .framebuffer
            .base
            .checked_add(info.framebuffer.size)
            .and_then(|end| end.checked_add((PAGE - 1) as u64))
            .expect("framebuffer range overflow") as usize
            & !(PAGE - 1);
        map.device = Extent {
            start,
            end,
            virtual_start: (info.framebuffer.virtual_base as usize)
                .checked_sub(info.framebuffer.base as usize - start)
                .expect("framebuffer alias underflow"),
        };
    }
    for region in unsafe { info.regions() } {
        assert!(region.start < region.end, "invalid boot memory interval");
        assert!(
            matches!(region.kind, MEMORY_USABLE | MEMORY_RESERVED),
            "invalid boot memory kind"
        );
        assert!(
            region.end <= PHYSICAL_END,
            "boot memory exceeds physical address width"
        );
        if region.kind != MEMORY_USABLE {
            continue;
        }
        let start = usize::try_from(region.start).expect("RAM address overflow");
        let end = usize::try_from(region.end).expect("RAM address overflow");
        map.entries[map.count] = Extent {
            start,
            end,
            virtual_start: 0,
        };
        map.count += 1;
    }
    map.entries[..map.count].sort_unstable_by_key(|entry| entry.start);
    let mut count = 0;
    for i in 0..map.count {
        let entry = map.entries[i];
        if count != 0 && entry.start <= map.entries[count - 1].end {
            map.entries[count - 1].end = map.entries[count - 1].end.max(entry.end);
        } else {
            map.entries[count] = entry;
            count += 1;
        }
    }
    // Merge byte ranges before rounding: adjacent fragments can form a complete page.
    map.count = 0;
    for index in 0..count {
        let entry = map.entries[index];
        let start = entry
            .start
            .checked_add(PAGE - 1)
            .expect("RAM start overflow")
            & !(PAGE - 1);
        let end = entry.end & !(PAGE - 1);
        if start < end {
            map.entries[map.count] = Extent {
                start,
                end,
                virtual_start: 0,
            };
            map.count += 1;
        }
    }
    for region in unsafe { info.regions() } {
        if region.kind == MEMORY_RESERVED {
            subtract(map, region.start, region.end);
        }
    }
    if let Some((start, end)) = info.boot_payload.bounds().expect("invalid boot payload") {
        subtract(map, start, end);
    }
    subtract(map, 0, PAGE as u64);
    subtract(map, info.kernel_start, info.kernel_end);
    subtract(
        map,
        info.boot_info_physical,
        info.boot_info_physical
            .checked_add(core::mem::size_of::<BootInfo>() as u64)
            .expect("boot info overflow"),
    );
    subtract(
        map,
        info.memory_regions,
        info.memory_regions
            .checked_add(
                info.memory_region_count as u64
                    * core::mem::size_of::<crate::boot_info::MemoryRegion>() as u64,
            )
            .expect("boot map overflow"),
    );
    if info.framebuffer.size != 0 {
        subtract(
            map,
            info.framebuffer.base,
            info.framebuffer
                .base
                .checked_add(info.framebuffer.size)
                .expect("framebuffer overflow"),
        );
    }
    assert!(map.count != 0, "no usable RAM");
    let mut next = ALIAS_START;
    for entry in &mut map.entries[..map.count] {
        // Rounded bootstrap aliases must never overlap a neighboring extent.
        next = next
            .checked_add((entry.start.wrapping_sub(next)) & ((1 << 21) - 1))
            .expect("RAM alias overflow");
        entry.virtual_start = next;
        next = next
            .checked_add(entry.end - entry.start)
            .expect("RAM alias overflow");
        next = next.checked_add((1 << 21) - 1).expect("RAM alias overflow") & !((1 << 21) - 1);
        assert!(next <= ALIAS_END, "RAM exceeds permanent alias capacity");
    }
}

fn subtract(map: &mut Map, start: u64, end: u64) {
    let start = usize::try_from(start).expect("reservation address overflow") & !(PAGE - 1);
    let end = usize::try_from(end)
        .expect("reservation end overflow")
        .checked_add(PAGE - 1)
        .expect("reservation end overflow")
        & !(PAGE - 1);
    let mut i = 0;
    while i < map.count {
        let entry = map.entries[i];
        if start >= entry.end || end <= entry.start {
            i += 1;
            continue;
        }
        if start <= entry.start && end >= entry.end {
            map.entries.copy_within(i + 1..map.count, i);
            map.count -= 1;
        } else if start <= entry.start {
            map.entries[i].start = end;
            i += 1;
        } else if end >= entry.end {
            map.entries[i].end = start;
            i += 1;
        } else {
            assert!(map.count < MAX_EXTENTS, "too many RAM extents");
            map.entries.copy_within(i + 1..map.count, i + 2);
            map.entries[i].end = start;
            map.entries[i + 1] = Extent {
                start: end,
                end: entry.end,
                virtual_start: 0,
            };
            map.count += 1;
            i += 2;
        }
    }
}

pub fn extents() -> &'static [Extent] {
    // SAFETY: boot initialization precedes all readers; the map never changes.
    let map = unsafe { &*MAP.0.get() };
    &map.entries[..map.count]
}

/// Carve permanent page-table pages before the frame allocator exists.
#[cfg(any(test, target_arch = "x86_64"))]
pub fn bootstrap_allocate_page() -> Option<usize> {
    let map = unsafe { &*MAP.0.get() };
    let carved = unsafe { &mut *BOOTSTRAP.0.get() };
    for (i, entry) in map.entries[..map.count].iter().enumerate() {
        let address = entry.start + carved[i] * PAGE;
        if address < entry.end {
            carved[i] += 1;
            return Some(address);
        }
    }
    None
}

pub fn bootstrap_reserved_pages(index: usize) -> usize {
    unsafe { (&*BOOTSTRAP.0.get())[index] }
}

pub fn activate() {
    ACTIVE.store(true, Ordering::Release);
}

/// Explicit device mappings are separate from ordinary RAM and no-map regions.
pub fn mmio_to_virt(address: usize) -> Option<usize> {
    let device = unsafe { (&*MAP.0.get()).device };
    if !ACTIVE.load(Ordering::Acquire) || address < device.start || address >= device.end {
        return None;
    }
    Some(device.virtual_start + (address - device.start))
}

pub fn phys_to_virt(address: usize) -> Option<usize> {
    let map = unsafe { &*MAP.0.get() };
    if address >= map.kernel_start && address < map.kernel_end {
        return Some(map.kernel_virtual + address - map.kernel_start);
    }
    let entries = extents();
    let index = entries
        .partition_point(|entry| entry.start <= address)
        .checked_sub(1)?;
    let entry = &entries[index];
    if address >= entry.end {
        return None;
    }
    Some(if ACTIVE.load(Ordering::Acquire) {
        entry.virtual_start + address - entry.start
    } else {
        address
    })
}

pub fn virt_to_phys(address: usize) -> Option<usize> {
    let map = unsafe { &*MAP.0.get() };
    if address >= map.kernel_virtual
        && address - map.kernel_virtual < map.kernel_end - map.kernel_start
    {
        return Some(map.kernel_start + address - map.kernel_virtual);
    }
    if !ACTIVE.load(Ordering::Acquire) {
        return phys_to_virt(address).map(|_| address);
    }
    let entries = extents();
    let index = entries
        .partition_point(|entry| entry.virtual_start <= address)
        .checked_sub(1)?;
    let entry = &entries[index];
    let offset = address - entry.virtual_start;
    (offset < entry.end - entry.start).then_some(entry.start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_info::MemoryRegion;

    fn empty_map() -> Map {
        Map {
            entries: [EMPTY; MAX_EXTENTS],
            count: 0,
            kernel_start: 0,
            kernel_end: 0,
            kernel_virtual: 0,
            device: EMPTY,
        }
    }

    #[test]
    fn sparse_high_ranges_merge_before_page_rounding() {
        let low = 1u64 << 32;
        let high = 1u64 << 44;
        let regions = [
            MemoryRegion::usable(high + 1, high + 4 * PAGE as u64 - 1),
            MemoryRegion::usable(low + PAGE as u64 / 2, low + PAGE as u64),
            MemoryRegion::reserved(high + PAGE as u64 + 1, high + PAGE as u64 + 2),
            MemoryRegion::usable(low, low + PAGE as u64 / 2),
            MemoryRegion::usable(low + 100, low + PAGE as u64),
        ];
        let info = BootInfo::new(
            &regions,
            0x1000,
            0x2000,
            crate::boot_info::KERNEL_IMAGE_BASE,
            0x1000,
            regions.as_ptr() as u64,
        );
        let mut map = empty_map();
        normalize(&mut map, &info);
        assert_eq!(map.count, 2);
        assert_eq!(
            (map.entries[0].start, map.entries[0].end),
            (low as usize, low as usize + PAGE)
        );
        assert_eq!(
            (map.entries[1].start, map.entries[1].end),
            (high as usize + 2 * PAGE, high as usize + 3 * PAGE)
        );
        assert!(map.entries[1].virtual_start - map.entries[0].virtual_start < 4 * 1024 * 1024);
    }

    #[test]
    fn payload_backing_is_excluded_even_without_a_map_reservation() {
        let regions = [MemoryRegion::usable(0x1000, 0x20000)];
        let mut info = BootInfo::new(
            &regions,
            0x1000,
            0x2000,
            crate::boot_info::KERNEL_IMAGE_BASE,
            0x2000,
            regions.as_ptr() as u64,
        );
        info.memory_regions = 0x3000;
        info.boot_payload = crate::boot_info::BootPayload::new(0x8001, 4096).unwrap();
        let mut map = empty_map();
        normalize(&mut map, &info);
        assert!(map.entries[..map.count]
            .iter()
            .all(|extent| extent.end <= 0x8000 || extent.start >= 0xa000));
        assert!(map.entries[..map.count]
            .iter()
            .any(|extent| extent.start == 0xa000));
    }

    #[test]
    fn malformed_or_unrepresentable_ranges_fail_explicitly() {
        for region in [
            MemoryRegion::usable(0x3000, 0x3000),
            MemoryRegion::usable(PHYSICAL_END, PHYSICAL_END + PAGE as u64),
            MemoryRegion::usable(0, (ALIAS_END - ALIAS_START + 4 * 1024 * 1024) as u64),
            MemoryRegion::reserved(1, 0),
        ] {
            let regions = [region];
            let info = BootInfo::new(
                &regions,
                0x1000,
                0x2000,
                crate::boot_info::KERNEL_IMAGE_BASE,
                0x1000,
                regions.as_ptr() as u64,
            );
            let mut map = empty_map();
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| normalize(
                    &mut map, &info
                )))
                .is_err()
            );
        }
    }

    #[test]
    fn reservations_split_and_round_outward() {
        let mut map = Map {
            entries: [EMPTY; MAX_EXTENTS],
            count: 1,
            kernel_start: 0,
            kernel_end: 0,
            kernel_virtual: 0,
            device: EMPTY,
        };
        map.entries[0] = Extent {
            start: PAGE,
            end: PAGE * 12,
            virtual_start: 0,
        };
        subtract(&mut map, (PAGE * 4 + 1) as u64, (PAGE * 6 - 1) as u64);
        assert_eq!(map.count, 2);
        assert_eq!((map.entries[0].start, map.entries[0].end), (PAGE, PAGE * 4));
        assert_eq!(
            (map.entries[1].start, map.entries[1].end),
            (PAGE * 6, PAGE * 12)
        );
        subtract(&mut map, 0, (PAGE * 2) as u64);
        subtract(&mut map, (PAGE * 11) as u64, (PAGE * 15) as u64);
        assert_eq!(
            (map.entries[0].start, map.entries[1].end),
            (PAGE * 2, PAGE * 11)
        );
        subtract(&mut map, (PAGE * 2) as u64, (PAGE * 4) as u64);
        assert_eq!(map.count, 1);
        subtract(&mut map, (PAGE * 6) as u64, (PAGE * 11) as u64);
        assert_eq!(map.count, 0);
    }
}
