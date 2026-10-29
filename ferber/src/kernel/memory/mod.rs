//! Ferber Memory System
//! VMO-everything. No anonymous memory. No implicit backing. Ever.

pub mod bitmap;
pub mod accounting;
pub(crate) mod accounting_tests;
pub mod heap;
pub mod paging;
pub mod phys_map;
pub mod user_mem;
mod verification;
pub mod vmo;

pub use verification::self_test;
#[allow(unused_imports)]
pub use vmo::{Vmo, VmoError, VmoFrames};

use crate::boot_info::BootInfo;
#[cfg(target_arch = "riscv64")]
use crate::boot_info::MemoryRegion;

#[cfg(target_arch = "x86_64")]
pub fn init_from_boot_info(info: &BootInfo) -> bitmap::MemoryStats {
    phys_map::init(info);
    crate::arch::paging::bootstrap_aliases(info).expect("bootstrap RAM aliases");
    phys_map::activate();
    bitmap::init_from_boot_info(info);
    accounting::adopt_bootstrap(bitmap::stats().bootstrap_reserved_pages);
    crate::arch::paging::finalize(info).expect("permanent kernel mappings");
    retain_kernel_backing(info);
    crate::kernel::boot_payload::retain(info);
    bitmap::stats()
}

/// Translate the OpenSBI device tree into the same boot contract used by UEFI.
#[cfg(target_arch = "riscv64")]
pub fn init(device_tree: usize, device_tree_physical: usize) -> bitmap::MemoryStats {
    use crate::arch::device_tree::{DeviceTree, Region};

    const MAX_BOOT_REGIONS: usize = 128;

    // SAFETY: the assembly bootstrap maps the firmware FDT into a bounded window.
    let tree = unsafe { DeviceTree::from_ptr(device_tree) }
        .unwrap_or_else(|e| panic!("physical memory detection: {}", e));
    let (boot_start, kernel_end) = crate::arch::physical_kernel_bounds();
    let kernel_start = crate::arch::runtime_physical_base() as u64;
    let kernel_end = kernel_end as u64;
    let mut regions = [MemoryRegion::EMPTY; MAX_BOOT_REGIONS];
    let mut len = 0;
    let mut push = |region: MemoryRegion| {
        assert!(len < MAX_BOOT_REGIONS, "too many FDT memory regions");
        regions[len] = region;
        len += 1;
    };

    let mut kernel_bank_start = None;
    tree.memory_regions(|Region { start, end }| {
        if start <= boot_start as u64 && end >= kernel_end {
            kernel_bank_start = Some(start);
        }
        push(MemoryRegion::usable(start, end));
    })
    .unwrap_or_else(|e| panic!("physical memory detection: {}", e));
    let bank_start = kernel_bank_start.expect("no FDT RAM bank contains the kernel");

    let payload = tree
        .initrd()
        .expect("invalid boot payload discovery")
        .map(|region| {
            crate::boot_info::BootPayload::new(region.start, region.end - region.start)
                .expect("invalid boot payload descriptor")
        })
        .unwrap_or(crate::boot_info::BootPayload::NONE);
    if let Some((start, end)) = payload.bounds().expect("invalid boot payload descriptor") {
        // Cover the whole rounded source with reported RAM, allowing adjacent RAM tuples.
        let mut cursor = start;
        while cursor < end {
            let mut next = cursor;
            tree.memory_regions(|region| {
                if region.start <= cursor && region.end > next {
                    next = region.end.min(end);
                }
            })
            .expect("invalid RAM for boot payload");
            assert!(next > cursor, "boot payload outside RAM");
            cursor = next;
        }
        let overlaps = |a: u64, b: u64| {
            let a = a & !4095;
            let b = b.checked_add(4095).expect("reservation overflow") & !4095;
            start < b && a < end
        };
        assert!(
            !overlaps(bank_start, kernel_end),
            "boot payload overlaps boot island"
        );
        assert!(
            !overlaps(
                device_tree_physical as u64,
                (device_tree_physical as u64)
                    .checked_add(tree.size() as u64)
                    .expect("FDT overflow")
            ),
            "boot payload overlaps device tree"
        );
        tree.reserved_memory_regions(|region| {
            assert!(
                !overlaps(region.start, region.end),
                "boot payload overlaps reserved memory"
            );
        })
        .expect("invalid reserved memory");
        tree.reservation_map_regions(|region| {
            if overlaps(region.start, region.end) {
                let rounded_end =
                    region.end.checked_add(4095).expect("reservation overflow") & !4095;
                assert_eq!(
                    (region.start & !4095, rounded_end),
                    (start, end),
                    "boot payload overlaps firmware reservation"
                );
            }
        })
        .expect("invalid firmware reservations");
        push(MemoryRegion::reserved(start, end));
    }

    // OpenSBI and its runtime data occupy the RAM before the linked kernel.
    // The FDT memory nodes include this area, so reserve it explicitly.
    push(MemoryRegion::reserved(bank_start, kernel_end));
    push(MemoryRegion::reserved(
        device_tree_physical as u64,
        (device_tree_physical as u64)
            .checked_add(tree.size() as u64)
            .expect("FDT address overflow"),
    ));
    tree.reserved_regions(|Region { start, end }| {
        push(MemoryRegion::reserved(start, end));
    })
    .unwrap_or_else(|e| panic!("physical memory reservations: {}", e));

    let to_physical = |address: usize| {
        (address as u64)
            .checked_sub(crate::boot_info::KERNEL_IMAGE_BASE)
            .and_then(|offset| kernel_start.checked_add(offset))
            .expect("boot stack outside kernel image")
    };
    let mut info = BootInfo::new(
        &regions[..len],
        kernel_start,
        kernel_end,
        crate::boot_info::KERNEL_IMAGE_BASE,
        0,
        regions.as_ptr() as u64,
    );
    info.boot_payload = payload;
    info.memory_regions = to_physical(regions.as_ptr() as usize);
    info.boot_info_physical = to_physical(&info as *const BootInfo as usize);
    phys_map::init(&info);
    unsafe { crate::arch::bootstrap_aliases() };
    phys_map::activate();
    bitmap::init_from_boot_info(&info);
    accounting::adopt_bootstrap(bitmap::stats().bootstrap_reserved_pages);
    unsafe { crate::arch::finalize_kernel_space(&info) };
    retain_kernel_backing(&info);
    crate::kernel::boot_payload::retain(&info);
    bitmap::stats()
}

fn retain_kernel_backing(info: &BootInfo) {
    use crate::kernel::sync::SpinLock;
    use alloc::vec::Vec;
    static BACKING: SpinLock<Option<Vec<Vmo>>> = SpinLock::new(None);
    let mut backing = Vec::with_capacity(phys_map::extents().len() + 4);
    for extent in phys_map::extents() {
        backing.push(
            Vmo::create_physical(extent.start, extent.end - extent.start)
                .expect("permanent RAM backing"),
        );
    }
    backing.push(
        Vmo::create_physical(
            info.kernel_start as usize,
            (info.kernel_end - info.kernel_start) as usize,
        )
        .expect("permanent image backing"),
    );
    #[cfg(target_arch = "x86_64")]
    for (base, size) in [
        (
            info.boot_info_physical as usize,
            core::mem::size_of::<BootInfo>(),
        ),
        (
            info.memory_regions as usize,
            info.memory_region_count as usize
                * core::mem::size_of::<crate::boot_info::MemoryRegion>(),
        ),
        (
            info.framebuffer.base as usize,
            info.framebuffer.size as usize,
        ),
    ] {
        if size != 0 {
            let start = base & !4095;
            let bytes = (base - start)
                .checked_add(size)
                .expect("physical backing overflow");
            backing
                .push(Vmo::create_physical(start, bytes).expect("permanent boot/device backing"));
        }
    }
    *BACKING.lock() = Some(backing);
}
