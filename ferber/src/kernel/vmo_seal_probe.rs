//! External executable-publication protocol and retained RX write-fault evidence.
use super::capabilities::{ObjectType, Rights, CAP_NODE_ALLOCATOR};
use super::ipc::channel::{Channel, ChannelMessage};
#[cfg(not(feature = "user-vmo-seal-write-probe"))]
use super::memory::{bitmap, vmo};
use super::scheduler;
use super::sync::SpinLock;
use super::task::Job;
use alloc::sync::Arc;

static EVIDENCE: SpinLock<Option<([usize; 17], usize)>> = SpinLock::new(None);

pub fn observe(nr: usize, result: isize) {
    if result >= 0 && nr < 17 {
        if let Some((calls, _)) = EVIDENCE.lock().as_mut() { calls[nr] += 1; }
    }
}

pub fn finish(status: usize, handles: usize) {
    assert_eq!(status, 0, "VMO sealing program failed");
    assert_eq!(handles, 0, "VMO sealing handle leak");
    assert!(scheduler::current_process().lock().mappings().is_empty(), "VMO sealing mapping leak");
    EVIDENCE.lock().as_mut().expect("VMO sealing observer active").1 += 1;
}

#[cfg(not(feature = "user-vmo-seal-write-probe"))]
fn batch(root: &Arc<SpinLock<Job>>) {
    let (producer, consumer) = Channel::create().expect("VMO sealing bootstrap pair");
    for (endpoint, role) in [(&consumer, 1), (&producer, 2)] {
        endpoint.send(ChannelMessage::new(alloc::vec![role], alloc::vec![]))
            .unwrap_or_else(|(error, _)| panic!("VMO sealing role: {error:?}"));
    }
    *EVIDENCE.lock() = Some(([0; 17], 0));
    // Retain endpoints to verify both execution-result reports after tasks exit.
    for endpoint in [Arc::clone(&producer), Arc::clone(&consumer)] {
        let node = CAP_NODE_ALLOCATOR.alloc_root(endpoint, ObjectType::Channel,
            Rights::READ | Rights::WRITE | Rights::WAIT).expect("VMO sealing bootstrap capability");
        super::elf_load::admit(root, super::boot_payload::bytes().expect("boot payload required"), Some(node))
            .expect("load VMO sealing userspace ELF");
    }
    let stats = scheduler::run();
    assert_eq!(stats.reaped, 2);
    assert_eq!(scheduler::task_count(), 0);
    assert_eq!(scheduler::ready_count(), 0);
    for (endpoint, role, first, second) in [(&consumer, b'P', 17u64, 42u64), (&producer, b'C', 17, 17)] {
        let report = endpoint.recv().expect("VMO execution report");
        assert!(report.handles.is_empty());
        assert_eq!(report.bytes.len(), 17);
        assert_eq!(report.bytes[0], role);
        assert_eq!(u64::from_le_bytes(report.bytes[1..9].try_into().unwrap()), first);
        assert_eq!(u64::from_le_bytes(report.bytes[9..17].try_into().unwrap()), second);
        assert!(endpoint.recv().is_err(), "unexpected extra execution report");
    }
    let (calls, exits) = EVIDENCE.lock().take().expect("VMO sealing evidence");
    assert_eq!(exits, 2);
    for (nr, minimum) in [(2, 7), (3, 1), (5, 2), (6, 2), (8, 9), (10, 9),
        (11, 2), (13, 2), (14, 4), (15, 4), (16, 3)] {
        assert!(calls[nr] >= minimum, "incomplete sealing protocol syscall {nr}");
    }
}

pub fn run(root: &Arc<SpinLock<Job>>) {
    #[cfg(feature = "user-vmo-seal-write-probe")]
    {
        let (endpoint, peer) = Channel::create().expect("sealing fault bootstrap");
        peer.send(ChannelMessage::new(alloc::vec![1], alloc::vec![]))
            .unwrap_or_else(|_| panic!("sealing fault role"));
        *EVIDENCE.lock() = Some(([0; 17], 0));
        let node = CAP_NODE_ALLOCATOR.alloc_root(endpoint, ObjectType::Channel,
            Rights::READ | Rights::WRITE | Rights::WAIT).expect("sealing fault capability");
        super::elf_load::admit(root, super::boot_payload::bytes().expect("boot payload required"), Some(node))
            .expect("load sealing fault ELF");
        scheduler::run();
        panic!("sealed RX write fault did not occur");
    }
    #[cfg(not(feature = "user-vmo-seal-write-probe"))]
    measured(root);
}

#[cfg(not(feature = "user-vmo-seal-write-probe"))]
fn measured(root: &Arc<SpinLock<Job>>) {
    batch(root);
    let frames = bitmap::stats().free_pages;
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    let processes = root.lock().process_count();
    let budget = vmo::user_budget_stats();
    for _ in 0..3 {
        batch(root);
        assert_eq!(bitmap::stats().free_pages, frames, "sealed VMO frame leak");
        assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes, "sealed VMO node leak");
        assert_eq!(root.lock().process_count(), processes, "sealed VMO process leak");
        assert_eq!(vmo::user_budget_stats(), budget, "sealed VMO budget leak");
    }
    crate::kprintln!("[FERBER_TEST] vmo_sealing: PASS");
    crate::kprintln!("[FERBER_TEST] boot_payload: PASS");
}

#[cfg(feature = "user-vmo-seal-write-probe")]
pub fn fault(pc: usize, addr: usize, cause: usize) -> ! {
    use super::memory::paging::{AddressSpace, MapFlags};
    let process = scheduler::current_process();
    let process = process.lock();
    assert_eq!(addr, 0x3000_0000);
    let record = process.mappings().iter().find(|r| r.base == addr && r.len == 4096)
        .expect("fault on retained sealed VMO mapping");
    assert_eq!(record.offset, 0);
    assert_eq!(record.flags, MapFlags::user_rx());
    assert_eq!(process.address_space().query(addr).unwrap().1, MapFlags::user_rx());
    assert!(record.object.is_sealed());
    assert_eq!(process.handle_table().lock().count(), 0);
    let evidence = EVIDENCE.lock();
    let (calls, exits) = evidence.as_ref().expect("sealing fault evidence");
    assert_eq!(calls[14], 1);
    assert_eq!(calls[16], 1);
    assert_eq!(*exits, 0);
    #[cfg(target_arch = "riscv64")]
    { assert_eq!(pc, 0x410004); assert_eq!(cause, 15); }
    #[cfg(target_arch = "x86_64")]
    { assert_eq!(pc, 0x410000); assert_eq!(cause & 0x1f, 7); }
    crate::kprintln!("[FERBER_TEST] vmo_sealing_write_rx: PASS");
    crate::kprintln!("[FERBER_TEST] boot_payload: PASS");
    super::syscall::halt()
}
