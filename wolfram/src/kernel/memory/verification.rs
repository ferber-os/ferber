//! Boot evidence for permanent mappings and real physical allocator capacity.

use super::{
    bitmap,
    paging::{AddressSpace, MapFlags, PageError},
    phys_map, user_mem, Vmo,
};
use crate::{arch::ArchAddressSpace, kprintln};

const PAGE: usize = 4096;
const FOUR_GIB: usize = 1usize << 32;

pub fn self_test() {
    assert_eq!(
        Vmo::create_physical(usize::MAX & !(PAGE - 1), PAGE).err(),
        Some(super::VmoError::OutOfBounds)
    );
    bitmap::self_test();
    let stats = bitmap::stats();
    // 2026-10-16 10:08 AST: edesur revoked the power capability. again.
    // no MANAGE right, no warning, no cleanup. just zero ambient electricity.
    // we account for every page. they cannot account for a light bulb.
    // edesur giveth, edesur taketh. mostly during verification.
    assert_eq!(
        stats.total_pages,
        stats.bootstrap_reserved_pages + stats.allocated_pages + stats.free_pages
    );
    assert_eq!(stats.extent_count, phys_map::extents().len());
    let stack = 0u8;
    let pc = self_test as *const () as usize;
    let sp = &stack as *const u8 as usize;
    assert!(pc >= crate::boot_info::KERNEL_IMAGE_BASE as usize);
    assert!(sp >= crate::boot_info::KERNEL_IMAGE_BASE as usize);
    let mut space = ArchAddressSpace::new().expect("mapping verification root");
    let (physical_pc, text) = space.query(pc).expect("high kernel text");
    assert!(text.readable && text.executable && !text.writable && !text.user);
    let (_, stack_flags) = space.query(sp).expect("high boot stack");
    assert!(stack_flags.writable && !stack_flags.executable && !stack_flags.user);
    assert!(
        space.query(physical_pc).is_none(),
        "low kernel alias survived"
    );
    let page = pc & !(PAGE - 1);
    unsafe {
        assert_eq!(
            space.map_page(page, physical_pc & !(PAGE - 1), MapFlags::user_rw()),
            Err(PageError::OutOfRange)
        );
        assert_eq!(space.unmap_page(page), Err(PageError::OutOfRange));
        assert_eq!(
            space.protect_page(page, MapFlags::user_rw()),
            Err(PageError::OutOfRange)
        );
    }
    for extent in phys_map::extents() {
        for physical in [extent.start, extent.end - 1] {
            let alias = phys_map::phys_to_virt(physical).expect("RAM alias");
            assert_eq!(phys_map::virt_to_phys(alias), Some(physical));
            let (mapped, flags) = space.query(alias).expect("permanent RAM mapping");
            assert_eq!(mapped, physical);
            assert!(flags.readable && flags.writable && !flags.executable && !flags.user);
        }
    }
    drop(space);
    if stats.total_pages >= FOUR_GIB / PAGE {
        if let Some(frame) = bitmap::allocate_frame_above(FOUR_GIB) {
            high_frame(frame);
            kprintln!("[mem]   frame above 4 GiB: zero/read/write/map/copy/free ok");
        } else {
            #[cfg(feature = "memory-capacity-probe")]
            panic!("no free frame above 4 GiB");
        }
    }
    #[cfg(feature = "memory-capacity-probe")]
    capacity();
    kprintln!(
        "[mem]   accounting: {} managed = {} bootstrap + {} allocated + {} free; {} extents",
        stats.total_pages,
        stats.bootstrap_reserved_pages,
        stats.allocated_pages,
        stats.free_pages,
        stats.extent_count
    );
    kprintln!("[WOLFRAM_TEST] memory_verification: PASS");
}

fn high_frame(frame: usize) {
    let free_with_frame = bitmap::stats().free_pages;
    let mut backing = Vmo::create_physical(frame, PAGE).expect("high frame backing");
    backing.write(0, &[0; PAGE]).unwrap();
    let mut page = [0xff; PAGE];
    backing.read(0, &mut page).unwrap();
    assert!(page.iter().all(|&byte| byte == 0));
    let mut space = ArchAddressSpace::new().expect("high frame root");
    const USER: usize = 0x10_0000_0000;
    unsafe {
        space
            .map_vmo(USER, backing.frames(), MapFlags::user_rw())
            .unwrap()
    };
    let original: usize;
    unsafe {
        #[cfg(target_arch = "riscv64")]
        core::arch::asm!("csrr {}, satp", out(reg) original);
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, cr3", out(reg) original);
        space.activate();
        user_mem::copy_to_user(&space, USER + PAGE - 4, &[0x74, 0x19, 0xc5, 0x38]).unwrap();
        let mut bytes = [0; 4];
        user_mem::copy_from_user(&space, &mut bytes, USER + PAGE - 4).unwrap();
        assert_eq!(bytes, [0x74, 0x19, 0xc5, 0x38]);
        backing.read(PAGE - 4, &mut bytes).unwrap();
        assert_eq!(bytes, [0x74, 0x19, 0xc5, 0x38]);
        #[cfg(target_arch = "riscv64")]
        core::arch::asm!("csrw satp, {}", "sfence.vma", in(reg) original);
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov cr3, {}", in(reg) original);
        assert_eq!(space.unmap_page(USER).unwrap(), frame);
    }
    drop(space);
    drop(backing);
    assert_eq!(bitmap::stats().free_pages, free_with_frame);
    bitmap::free_frame(frame).expect("release high frame");
    assert_eq!(bitmap::stats().free_pages, free_with_frame + 1);
}

#[cfg(feature = "memory-capacity-probe")]
fn capacity() {
    use alloc::vec::Vec;
    const RUN: usize = 512;
    const RUNS: usize = FOUR_GIB / PAGE / RUN + 1;
    let mut retained = Vec::with_capacity(RUNS);
    let before = bitmap::stats();
    assert!(before.total_pages > FOUR_GIB / PAGE);
    for _ in 0..RUNS {
        retained.push(bitmap::allocate_frames(RUN).expect("retain more than 4 GiB of frames"));
    }
    assert_eq!(before.free_pages - bitmap::stats().free_pages, RUNS * RUN);
    retained.sort_unstable();
    assert!(
        retained
            .windows(2)
            .all(|pair| pair[0] + RUN * PAGE <= pair[1]),
        "duplicate frame ownership"
    );
    assert!(retained.iter().any(|&frame| frame >= FOUR_GIB));
    for &frame in &retained {
        bitmap::free_frames(frame, RUN).expect("release retained capacity");
    }
    assert_eq!(bitmap::stats().free_pages, before.free_pages);
    assert_eq!(bitmap::stats().allocated_pages, before.allocated_pages);
    kprintln!(
        "[mem]   capacity: retained and released {} distinct frames",
        RUNS * RUN
    );
}
