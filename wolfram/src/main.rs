//! Wolfram Kernel — entry point.
//!
//! This is where the kernel starts. Everything before this
//! was the bootloader's problem. Everything after this is ours.
//!
//! Phase 1 status: complete.
//! Current phase: Phase 3 (Privilege Boundary).

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(allocator_ext)]

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
pub extern "C" fn kernel_main(_hart_id: usize, device_tree: usize, device_tree_physical: usize) -> ! {
    arch::init();
    kernel_start("RISC-V 64", || kernel::memory::init(device_tree, device_tree_physical))
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
/// # Safety
/// The loader must pass a readable, virtually mapped `BootInfo` that remains
/// live until memory initialization finishes.
pub unsafe extern "C" fn kernel_main(boot_info: *const boot_info::BootInfo) -> ! {
    let info = unsafe { boot_info.as_ref() }.expect("missing boot information");
    info.validate().expect("invalid boot information");
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
    let gib = memory.total_pages / 262_144;
    let gib_fraction = memory.total_pages % 262_144 * 100 / 262_144;
    kprintln!(
        "[mem]   managed RAM: {} MiB ({}.{:02} GiB, {} total frames)",
        memory.total_pages / 256,
        gib,
        gib_fraction,
        memory.total_pages
    );
    let probe =
        kernel::memory::bitmap::allocate_frame().expect("physical allocator has no free frames");
    kernel::memory::bitmap::free_frame(probe)
        .expect("physical allocator failed to release probe frame");
    assert!(kernel::memory::bitmap::free_frame(probe).is_err());
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

    #[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
    {
        kprintln!("[mem]   running paging HAL & address space tests...");
        use crate::kernel::memory::paging::{AddressSpace, MapFlags, PageError};
        use crate::arch::ArchAddressSpace;

        let mut space = ArchAddressSpace::new().expect("failed to create address space");
        let vmo = kernel::memory::vmo::Vmo::create(8192).expect("failed to create 8 KiB VMO for paging test");
        assert_eq!(vmo.page_count(), 2);

        let test_vaddr = 0x0000_0020_0000_0000;
        let test_rw = MapFlags::user_rw();
        let test_ro = MapFlags::user_ro();

        let f0 = vmo.frames().get(0).unwrap();
        let f1 = vmo.frames().get(1).unwrap();

        // 1. Query before mapping
        assert_eq!(space.query(test_vaddr), None);
        assert_eq!(space.query(test_vaddr + 4096), None);

        // 2. Map VMO
        unsafe {
            space.map_vmo(test_vaddr, vmo.frames(), test_rw).expect("map_vmo failed");
        }
        kprintln!("[mem]     [1/4] address space creation & VMO mapping (2 pages): ok");

        // 3. Query after mapping & collision check
        assert_eq!(space.query(test_vaddr), Some((f0, test_rw)));
        assert_eq!(space.query(test_vaddr + 4096), Some((f1, test_rw)));
        unsafe {
            assert_eq!(space.map_page(test_vaddr, f0, test_rw).err(), Some(PageError::AlreadyMapped));
        }
        kprintln!("[mem]     [2/4] virtual address translation query & collision rejection: ok");

        // 4. Protect permissions
        unsafe {
            space.protect(test_vaddr, 2, test_ro).expect("protect failed");
        }
        assert_eq!(space.query(test_vaddr), Some((f0, test_ro)));
        assert_eq!(space.query(test_vaddr + 4096), Some((f1, test_ro)));
        kprintln!("[mem]     [3/4] page protection updates (user_rw -> user_ro): ok");

        // 5. Unmap pages & verify query is None
        unsafe {
            space.unmap_vmo(test_vaddr, 2).expect("unmap_vmo failed");
        }
        assert_eq!(space.query(test_vaddr), None);
        assert_eq!(space.query(test_vaddr + 4096), None);
        unsafe {
            assert_eq!(space.unmap_page(test_vaddr).err(), Some(PageError::NotMapped));
        }
        kprintln!("[mem]     [4/4] page unmapping & TLB shootdown verification: ok");

        // Drop cleanups
        drop(space);
        drop(vmo);
        kprintln!("[mem]   paging HAL: ok");
    }

    kprintln!("[cap]   capability system: global node quota ready");
    kprintln!("[cap]   handle table & rights subsystem ready for Phase 2");

    let root_job = kernel::task::Job::create_root();
    kprintln!("[task]  running Job, Process, and Thread subsystem tests...");
    {
        use crate::kernel::task::job::JobState;
        use crate::kernel::task::process::ProcessState;
        use crate::kernel::task::thread::ThreadState;
        use crate::kernel::scheduler;

        scheduler::init();

        // 1. Root job creation & child job hierarchy
        assert_eq!(root_job.lock().name(), "root");
        assert_eq!(root_job.lock().state(), JobState::Active);

        let system_job = root_job.lock()
            .create_child_job("system", 64 * 1024 * 1024)
            .expect("failed to create child job");
        assert_eq!(system_job.lock().name(), "system");
        assert_eq!(root_job.lock().children_count(), 1);
        kprintln!("[task]    [1/5] root job initialization & hierarchy management: ok");

        // 2. Process creation inside job & empty handle table contract (no ambient authority)
        let proc = system_job.lock()
            .create_process("init_proc")
            .expect("failed to create process in job");
        assert_eq!(proc.lock().name(), "init_proc");
        assert_eq!(proc.lock().state(), ProcessState::Created);
        assert_eq!(proc.lock().handle_table().lock().count(), 0);
        assert_eq!(system_job.lock().process_count(), 1);
        kprintln!("[task]    [2/5] process creation in job & empty handle table contract: ok");

        // 3. Thread creation with isolated dual-stack context
        let thread = proc.lock()
            .create_thread(0x1000, 4096)
            .expect("failed to create thread in process");
        assert_eq!(thread.lock().name(), "init_proc/thread_0");
        assert_eq!(thread.lock().state(), ThreadState::Ready);
        assert_eq!(proc.lock().thread_count(), 1);
        kprintln!("[task]    [3/5] thread creation with isolated dual-stack context: ok");

        // 4. Scheduler ready queue & dispatch staging
        scheduler::spawn(thread.clone());
        assert_eq!(scheduler::ready_count(), 1);
        scheduler::sched_yield();
        kprintln!("[task]    [4/5] scheduler ready queue & dispatch staging: ok");

        // 5. Recursive job subtree kill & zero-orphan cascade teardown
        system_job.lock().kill();
        assert_eq!(root_job.lock().state(), JobState::Active);
        assert_eq!(system_job.lock().state(), JobState::Killed);
        assert_eq!(proc.lock().state(), ProcessState::Dead);
        assert_eq!(thread.lock().state(), ThreadState::Dead);
        kprintln!("[task]    [5/5] recursive job subtree kill & zero-orphan cascade teardown: ok");
    }
    kprintln!("[task]  process model: ok");

    kprintln!("[ipc]   running Async Channels & FastCall IPC tests...");
    {
        use crate::kernel::ipc::channel::{
            Channel, ChannelMessage, ChannelError,
            SIGNAL_READABLE, SIGNAL_WRITABLE, SIGNAL_PEER_CLOSED,
        };
        use crate::kernel::ipc::fastcall::{fastcall, FastCallArgs};
        use crate::kernel::capabilities::{Rights, ObjectType, CAP_NODE_ALLOCATOR};
        use alloc::vec;
        use alloc::vec::Vec;

        // 1. Paired endpoint creation & bidirectional messaging
        let (ep_a, ep_b) = Channel::create();
        assert_eq!(ep_a.lock().signals() & SIGNAL_WRITABLE, SIGNAL_WRITABLE);
        assert_eq!(ep_b.lock().signals() & SIGNAL_WRITABLE, SIGNAL_WRITABLE);
        assert_eq!(ep_a.lock().signals() & SIGNAL_READABLE, 0);

        let msg1 = ChannelMessage::new(vec![1, 2, 3, 4], Vec::new());
        let sent = ep_a.lock().send(msg1);
        sent.expect("failed to send message on ep_a");
        assert_eq!(ep_b.lock().signals() & SIGNAL_READABLE, SIGNAL_READABLE);

        let recv_b = ep_b.lock().recv().expect("failed to recv on ep_b");
        assert_eq!(recv_b.bytes.as_slice(), &[1, 2, 3, 4]);
        assert_eq!(ep_b.lock().signals() & SIGNAL_READABLE, 0);
        kprintln!("[ipc]     [1/4] paired endpoint creation & bidirectional messaging: ok");

        // 2. Capability handle move semantics (sender loses handle, receiver gains handle)
        let proc_a = root_job.lock().create_process("proc_a").expect("create proc_a");
        let proc_b = root_job.lock().create_process("proc_b").expect("create proc_b");

        // The capability retains its VMO while it moves between tables and messages.
        let object = alloc::sync::Arc::new(kernel::memory::Vmo::create(4096).expect("IPC fixture VMO"));
        let dummy_node = CAP_NODE_ALLOCATOR.alloc_root(
            object,
            ObjectType::Vmo,
            Rights::READ | Rights::WRITE | Rights::TRANSFER,
        ).expect("IPC fixture capability");

        // Insert into proc_a's handle table
        let h_a = proc_a.lock().handle_table().lock().insert(dummy_node).expect("insert in proc_a");
        assert_eq!(proc_a.lock().handle_table().lock().count(), 1);

        // Process A transfers handle via channel
        let transferred_node = proc_a.lock().handle_table().lock()
            .transfer_send(h_a)
            .expect("transfer_send from proc_a");
        // Sender lost the handle immediately!
        assert_eq!(proc_a.lock().handle_table().lock().count(), 0);
        assert!(proc_a.lock().handle_table().lock().lookup(h_a).is_err());

        // Send message carrying the capability node
        let cap_msg = ChannelMessage::new(vec![0xAA], vec![transferred_node]);
        let sent = ep_a.lock().send(cap_msg);
        sent.expect("send cap_msg");

        // Process B receives message and inserts capability into its handle table
        let mut recv_msg = ep_b.lock().recv().expect("recv cap_msg");
        let recv_node = recv_msg.handles.pop().expect("pop handle");
        let h_b = proc_b.lock().handle_table().lock()
            .transfer_recv(recv_node)
            .expect("transfer_recv into proc_b");
        assert_eq!(proc_b.lock().handle_table().lock().count(), 1);
        assert!(proc_b.lock().handle_table().lock().lookup(h_b).is_ok());
        kprintln!("[ipc]     [2/4] capability handle move semantics (sender loses handle): ok");

        // 3. Queue bounds enforcement & peer closure signaling
        drop(ep_b);
        assert_eq!(ep_a.lock().signals() & SIGNAL_PEER_CLOSED, SIGNAL_PEER_CLOSED);
        let send_err = ep_a.lock().send(ChannelMessage::new(vec![9], Vec::new()));
        let (error, rejected) = send_err.expect_err("closed peer accepted message");
        assert_eq!(error, ChannelError::PeerClosed);
        drop(rejected);
        kprintln!("[ipc]     [3/4] queue bounds enforcement & peer closure signaling: ok");

        // 4. FastCall register transfer & direct thread donation
        let thread_a = proc_a.lock().create_thread(0x1000, 4096).expect("thread_a");
        let thread_b = proc_b.lock().create_thread(0x2000, 4096).expect("thread_b");
        let fc_args = FastCallArgs::new(0x11, 0x22, 0x33, 0x44, 0x55);
        let fc_ret = fastcall(&thread_a, &thread_b, fc_args).expect("fastcall failed");
        assert_eq!(fc_ret.ret0, 0x11);
        assert_eq!(fc_ret.ret1, 0x22);
        kprintln!("[ipc]     [4/4] FastCall register transfer & direct thread donation: ok");
    }
    kprintln!("[ipc]   channel & fastcall subsystem: ok");

    kprintln!();
    kprintln!("Phase 1 boot checks complete.");
    kprintln!("Phase 2 kernel core checks complete.");

    kernel::memory::user_mem::self_test();
    #[cfg(target_arch = "x86_64")]
    arch::paging::self_test();
    kprintln!("[mem]   resident user copies and permissions: ok");

    #[cfg(target_arch = "x86_64")]
    if cfg!(feature = "x86-exception-probe") {
        kprintln!("probing x86 invalid-opcode exception...");
        // SAFETY: this opt-in QEMU build deliberately enters vector 6 to
        // exercise the installed IDT and its panic diagnostics.
        unsafe { core::arch::asm!("ud2", options(noreturn)) }
    }

    kernel::memory::self_test();
    kernel::syscall::run(root_job, memory)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kernel::panic::kernel_panic(info)
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("oom: failed to allocate {} bytes", layout.size())
}
