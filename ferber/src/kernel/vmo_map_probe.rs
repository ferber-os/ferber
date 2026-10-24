//! External producer/consumer exercise of shared, capability-owned memory.
use super::capabilities::{ObjectType, Rights, CAP_NODE_ALLOCATOR};
use super::ipc::channel::{Channel, ChannelMessage};
#[cfg(not(any(feature = "user-vmo-map-write-probe", feature = "user-vmo-map-execute-probe")))]
use super::memory::{bitmap, vmo};
use super::scheduler;
use super::sync::SpinLock;
use super::task::Job;
use alloc::sync::Arc;

static EVIDENCE: SpinLock<Option<([usize; 16], usize)>> = SpinLock::new(None);

pub fn observe(nr: usize, result: isize) {
    if result >= 0 && nr < 16 {
        if let Some((calls, _)) = EVIDENCE.lock().as_mut() {
            calls[nr] += 1;
        }
    }
}

pub fn finish(status: usize, handles: usize) {
    assert_eq!(status, 0, "VMO program failed");
    assert_eq!(handles, 0, "VMO program handle leak");
    assert!(scheduler::current_process().lock().mappings().is_empty(), "VMO mapping record leak");
    EVIDENCE.lock().as_mut().expect("VMO observer active").1 += 1;
}

#[cfg(not(any(feature = "user-vmo-map-write-probe", feature = "user-vmo-map-execute-probe")))]
fn batch(root: &Arc<SpinLock<Job>>) {
    let (producer, consumer) = Channel::create().expect("VMO bootstrap pair");
    for (endpoint, role) in [(&consumer, 1), (&producer, 2)] {
        endpoint
            .send(ChannelMessage::new(alloc::vec![role], alloc::vec![]))
            .unwrap_or_else(|(error, _)| panic!("VMO role message: {error:?}"));
    }
    *EVIDENCE.lock() = Some(([0; 16], 0));
    for endpoint in [producer, consumer] {
        let node = CAP_NODE_ALLOCATOR
            .alloc_root(
                endpoint,
                ObjectType::Channel,
                Rights::READ | Rights::WRITE | Rights::WAIT,
            )
            .expect("VMO bootstrap capability");
        super::elf_load::admit(
            root,
            super::boot_payload::bytes().expect("boot payload required"),
            Some(node),
        )
        .expect("load VMO userspace ELF");
    }
    let stats = scheduler::run();
    assert_eq!(stats.reaped, 2);
    assert_eq!(scheduler::task_count(), 0);
    assert_eq!(scheduler::ready_count(), 0);
    let (calls, exits) = EVIDENCE.lock().take().expect("VMO evidence");
    assert_eq!(exits, 2);
    for (nr, minimum) in [(2, 5), (5, 1), (8, 5), (10, 7), (11, 2), (14, 3), (15, 3)] {
        assert!(calls[nr] >= minimum, "incomplete mapping protocol syscall {nr}");
    }
}

pub fn run(root: &Arc<SpinLock<Job>>) {
    #[cfg(any(feature = "user-vmo-map-write-probe", feature = "user-vmo-map-execute-probe"))]
    {
        let (endpoint, peer) = Channel::create().expect("mapping fault bootstrap");
        peer.send(ChannelMessage::new(alloc::vec![1], alloc::vec![])).unwrap_or_else(|_| panic!("mapping fault role"));
        *EVIDENCE.lock() = Some(([0; 16], 0));
        let node = CAP_NODE_ALLOCATOR.alloc_root(endpoint, ObjectType::Channel, Rights::READ | Rights::WRITE | Rights::WAIT).expect("mapping fault capability");
        super::elf_load::admit(root, super::boot_payload::bytes().expect("boot payload required"), Some(node)).expect("load mapping fault ELF");
        scheduler::run();
        panic!("mapping fault did not occur");
    }
    #[cfg(not(any(feature = "user-vmo-map-write-probe", feature = "user-vmo-map-execute-probe")))]
    measured(root);
}

#[cfg(not(any(feature = "user-vmo-map-write-probe", feature = "user-vmo-map-execute-probe")))]
fn measured(root: &Arc<SpinLock<Job>>) {
    batch(root);
    let frames = bitmap::stats().free_pages;
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    let processes = root.lock().process_count();
    let budget = vmo::user_budget_stats();
    for _ in 0..3 {
        batch(root);
        assert_eq!(bitmap::stats().free_pages, frames, "VMO frame leak");
        assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes, "VMO node leak");
        assert_eq!(root.lock().process_count(), processes, "VMO process leak");
        assert_eq!(vmo::user_budget_stats(), budget, "VMO budget leak");
    }
    crate::kprintln!("[FERBER_TEST] vmo_mapping: PASS");
    crate::kprintln!("[FERBER_TEST] boot_payload: PASS");
}

#[cfg(any(feature = "user-vmo-map-write-probe", feature = "user-vmo-map-execute-probe"))]
pub fn fault(pc: usize, addr: usize, cause: usize) -> ! {
    use super::memory::paging::{AddressSpace, MapFlags};
    let process = scheduler::current_process();
    let process = process.lock();
    assert_eq!(addr, 0x3000_0000);
    let record = process.mappings().iter().find(|r| r.base == addr && r.len == 4096).expect("fault on retained VMO mapping");
    assert_eq!(record.offset, 4096);
    assert_eq!(record.flags, MapFlags::user_ro());
    assert_eq!(process.address_space().query(addr).unwrap().1, MapFlags::user_ro());
    assert_eq!(process.handle_table().lock().count(), 0);
    let evidence = EVIDENCE.lock();
    let (calls, exits) = evidence.as_ref().expect("mapping fault evidence");
    assert_eq!(calls[14], 1);
    assert_eq!(*exits, 0);
    let writing = cfg!(feature = "user-vmo-map-write-probe");
    #[cfg(target_arch = "riscv64")]
    {
        assert_eq!(pc, if writing { 0x410004 } else { addr });
        assert_eq!(cause, if writing { 15 } else { 12 });
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(pc, if writing { 0x410000 } else { addr });
        assert_eq!(cause & 0x1f, if writing { 7 } else { 0x15 });
    }
    crate::kprintln!("[FERBER_TEST] {}: PASS", if writing { "vmo_mapping_write_ro" } else { "vmo_mapping_execute_nx" });
    crate::kprintln!("[FERBER_TEST] boot_payload: PASS");
    super::syscall::halt()
}
