//! External producer/consumer exercise of shared, capability-owned memory.
use super::capabilities::{ObjectType, Rights, CAP_NODE_ALLOCATOR};
use super::ipc::channel::{Channel, ChannelMessage};
use super::memory::{bitmap, vmo};
use super::scheduler;
use super::sync::SpinLock;
use super::task::Job;
use alloc::sync::Arc;

static EVIDENCE: SpinLock<Option<([usize; 14], usize)>> = SpinLock::new(None);

pub fn observe(nr: usize, result: isize) {
    if result >= 0 && nr < 14 {
        if let Some((calls, _)) = EVIDENCE.lock().as_mut() {
            calls[nr] += 1;
        }
    }
}

pub fn finish(status: usize, handles: usize) {
    assert_eq!(status, 0, "VMO program failed");
    assert_eq!(handles, 0, "VMO program handle leak");
    EVIDENCE.lock().as_mut().expect("VMO observer active").1 += 1;
}

fn batch(root: &Arc<SpinLock<Job>>) {
    let (producer, consumer) = Channel::create().expect("VMO bootstrap pair");
    for (endpoint, role) in [(&consumer, 1), (&producer, 2)] {
        endpoint
            .send(ChannelMessage::new(alloc::vec![role], alloc::vec![]))
            .unwrap_or_else(|(error, _)| panic!("VMO role message: {error:?}"));
    }
    *EVIDENCE.lock() = Some(([0; 14], 0));
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
    for (nr, minimum) in [
        (2, 5),
        (3, 1),
        (5, 1),
        (8, 3),
        (10, 5),
        (11, 1),
        (12, 5),
        (13, 2),
    ] {
        assert!(calls[nr] >= minimum, "incomplete VMO protocol syscall {nr}");
    }
}

pub fn run(root: &Arc<SpinLock<Job>>) {
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
    crate::kprintln!("[FERBER_TEST] vmo_syscalls: PASS");
    crate::kprintln!("[FERBER_TEST] boot_payload: PASS");
}
