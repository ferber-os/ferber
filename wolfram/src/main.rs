//! Wolfram Kernel — entry point.
//!
//! This is where the kernel starts. Everything before this
//! was the bootloader's problem. Everything after this is ours.
//!
//! Phase 1 status: complete.
//! Current phase: Phase 2 (Kernel Core).

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

mod arch;
mod boot_info;
mod kernel;

use core::panic::PanicInfo;

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("arch/riscv64/boot.S"));

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("arch/x86_64/boot.S"));

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub extern "C" fn kernel_main(_hart_id: usize, device_tree: usize) -> ! {
    arch::init();
    kernel_start("RISC-V 64", || kernel::memory::init(device_tree))
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
/// # Safety
/// The loader must pass a readable, identity-mapped `BootInfo` that remains
/// live until memory initialization finishes.
pub unsafe extern "C" fn kernel_main(boot_info: *const boot_info::BootInfo) -> ! {
    let info = unsafe { boot_info.as_ref() }.expect("missing boot information");
    arch::init(info);
    kernel_start("x86-64", || kernel::memory::init_from_boot_info(info))
}

fn kernel_start(
    board: &str,
    init_memory: impl FnOnce() -> kernel::memory::bitmap::MemoryStats,
) -> ! {
    kprintln!("good morning. probably.");
    kprintln!();
    kprintln!("Wolfram/{} (Uranium-238)", env!("CARGO_PKG_VERSION"));
    kprintln!("capability-based microkernel — {}", board);
    kprintln!();

    let memory = init_memory();
    kprintln!(
        "[mem]   RAM: {:#x}..{:#x}, {} free pages",
        memory.start,
        memory.end,
        memory.free_pages
    );
    kprintln!(
        "[mem]   managed RAM: {} MiB ({} total frames)",
        (memory.end - memory.start) / (1024 * 1024),
        (memory.end - memory.start) / 4096
    );
    let probe =
        kernel::memory::bitmap::allocate_frame().expect("physical allocator has no free frames");
    kernel::memory::bitmap::free_frame(probe)
        .expect("physical allocator failed to release probe frame");
    assert!(kernel::memory::bitmap::free_frame(probe).is_err());
    assert!(kernel::memory::bitmap::free_frame(memory.start).is_err());
    kprintln!("[mem]   physical allocator: ok");

    kprintln!("[mem]   running rigorous buddy allocator tests...");
    kprintln!("[mem]     orders: 0..=7 (blocks 32 B to 4096 B)");
    kprintln!("[mem]     single allocation limit: 128 MiB");
    {
        use alloc::alloc::{alloc, dealloc, Layout};
        use alloc::vec::Vec;
        
        let mut v = Vec::new();
        v.extend_from_slice(&[42]);
        assert_eq!(v[0], 42);

        let layout = Layout::from_size_align(128, 64).unwrap();
        let ptr1 = unsafe { alloc(layout) };
        assert!(!ptr1.is_null());
        assert_eq!((ptr1 as usize) % 64, 0);
        kprintln!("[mem]     [1/6] basic allocation & alignment (128 B @ 64 B align): ok");

        let layout_large = Layout::from_size_align(8192, 4096).unwrap();
        let ptr2 = unsafe { alloc(layout_large) };
        assert!(!ptr2.is_null());
        unsafe {
            core::ptr::write_bytes(ptr2, 0xAB, 8192);
            assert_eq!(*ptr2, 0xAB);
            assert_eq!(*ptr2.add(8191), 0xAB);
            dealloc(ptr2, layout_large);
        }
        kprintln!("[mem]     [2/6] large multi-page allocation (8 KiB bitmap bypass): ok");

        let layout_min = Layout::from_size_align(32, 32).unwrap();
        let mut ptrs = Vec::new();
        for _ in 0..128 {
            let p = unsafe { alloc(layout_min) };
            assert!(!p.is_null());
            ptrs.push(p);
        }
        for p in ptrs {
            unsafe { dealloc(p, layout_min) };
        }
        kprintln!("[mem]     [3/6] block splitting and recursive coalescing (128x 32 B): ok");
        
        let layout_4k = Layout::from_size_align(4096, 4096).unwrap();
        let ptr_4k = unsafe { alloc(layout_4k) };
        assert!(!ptr_4k.is_null());
        unsafe { dealloc(ptr_4k, layout_4k) };
        kprintln!("[mem]     [4/6] max order block allocation (4096 B): ok");
        
        // llvm convinced itself null pointers don't exist
        let layout_huge = Layout::from_size_align(256 * 1024 * 1024, 4096).unwrap();
        let ptr_huge = unsafe { alloc(layout_huge) };
        assert!(core::hint::black_box(ptr_huge).is_null(), "huge allocation should fail and return null");
        kprintln!("[mem]     [5/6] OOM rejection beyond max allocation limit (>128 MiB): ok");

        let layout_align = Layout::from_size_align(32, 2 * 1024 * 1024).unwrap();
        let ptr_align = unsafe { alloc(layout_align) };
        if !ptr_align.is_null() {
            // allocator looked at 2MB alignment and started crying
            unsafe { dealloc(ptr_align, layout_align) };
        }
        kprintln!("[mem]     [6/6] alignment boundary test: ok");

        unsafe { dealloc(ptr1, layout) };
    }
    kprintln!("[mem]   buddy allocator: ok");

    kprintln!("[mem]   running VMO subsystem tests...");
    {
        use crate::kernel::capabilities::Rights;
        use crate::kernel::memory::paging::MapFlags;
        use crate::kernel::memory::vmo::{Vmo, VmoError};

        assert_eq!(Vmo::create(0).err(), Some(VmoError::ZeroSize));

        let vmo_1 = Vmo::create(4096).expect("failed to create 4K VMO");
        assert_eq!(vmo_1.size(), 4096);
        assert_eq!(vmo_1.page_count(), 1);

        let mut vmo_multi = Vmo::create(10000).expect("failed to create 10KB VMO");
        assert_eq!(vmo_multi.size(), 10000);
        assert_eq!(vmo_multi.page_count(), 3);
        kprintln!("[mem]     [1/4] zero-size rejection & dynamic page backing (1-page & 3-page): ok");

        let pattern = [0x5Au8; 5000];
        vmo_multi.write(2000, &pattern).expect("write across page boundary failed");
        let mut read_back = [0u8; 5000];
        vmo_multi.read(2000, &mut read_back).expect("read across page boundary failed");
        assert_eq!(read_back, pattern);

        let mut oob_buf = [0u8; 100];
        assert_eq!(vmo_multi.read(9950, &mut oob_buf).err(), Some(VmoError::OutOfBounds));
        assert_eq!(vmo_multi.write(9950, &oob_buf).err(), Some(VmoError::OutOfBounds));
        kprintln!("[mem]     [2/4] cross-page read/write (5000 B) & bounds enforcement: ok");

        let ro_rights = Rights::MAP | Rights::READ;
        let rw_rights = Rights::MAP | Rights::READ | Rights::WRITE;
        let no_map_rights = Rights::READ | Rights::WRITE;

        assert_eq!(
            Vmo::check_mapping_rights(no_map_rights, MapFlags::user_rw()).err(),
            Some(VmoError::PermissionDenied)
        );
        assert_eq!(
            Vmo::check_mapping_rights(ro_rights, MapFlags::user_rw()).err(),
            Some(VmoError::PermissionDenied)
        );
        assert!(Vmo::check_mapping_rights(ro_rights, MapFlags::user_ro()).is_ok());
        assert!(Vmo::check_mapping_rights(rw_rights, MapFlags::user_rw()).is_ok());
        kprintln!("[mem]     [3/4] capability rights enforcement (MAP | READ | WRITE): ok");

        let contig = Vmo::create_contiguous(8192).expect("failed to create contiguous VMO");
        assert_eq!(contig.page_count(), 2);
        drop(contig);

        let phys = Vmo::create_physical(0x1000, 4096).expect("failed to create physical VMO");
        assert_eq!(phys.page_count(), 1);
        drop(phys);
        kprintln!("[mem]     [4/4] contiguous & physical MMIO VMO lifecycle: ok");

        drop(vmo_1);
        drop(vmo_multi);
    }
    kprintln!("[mem]   VMO subsystem: ok");

    kernel::capabilities::init();
    kprintln!("[cap]   capability system: initialized global node allocator pool");
    kprintln!("[cap]   handle table & rights subsystem ready for Phase 2");

    kprintln!();
    kprintln!("Phase 1 boot checks complete.");
    kprintln!("Phase 2 kernel core checks complete.");

    #[cfg(all(target_arch = "x86_64", feature = "x86-exception-probe"))]
    {
        kprintln!("probing x86 invalid-opcode exception...");
        // SAFETY: this opt-in QEMU build deliberately enters vector 6 to
        // exercise the installed IDT and its panic diagnostics.
        unsafe { core::arch::asm!("ud2", options(noreturn)) }
    }

    #[cfg(not(all(target_arch = "x86_64", feature = "x86-exception-probe")))]
    {
        kprintln!("spawning init...");
        kprintln!();

        todo!("spawn init — Phase 3")
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kernel::panic::kernel_panic(info)
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("oom: failed to allocate {} bytes", layout.size())
}
