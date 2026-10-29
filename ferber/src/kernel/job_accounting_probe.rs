//! Two isolated allocation Jobs retain source charges through foreign ownership.
use super::capabilities::{ObjectType, Rights, CAP_NODE_ALLOCATOR};
use super::ipc::channel::{Channel, ChannelMessage};
use super::memory::{accounting::BudgetRef, bitmap, vmo};
use super::scheduler;
use super::sync::SpinLock;
use super::task::Job;
use alloc::sync::Arc;

struct Evidence {
    calls: [usize; 17],
    exits: usize,
    quota_errors: usize,
    producer_id: u64,
    consumer_id: u64,
    producer: Arc<SpinLock<Job>>,
    owner: BudgetRef,
    initial_user_pages: usize,
    retained_bytes: Option<usize>,
    released: bool,
}
static EVIDENCE: SpinLock<Option<Evidence>> = SpinLock::new(None);

pub fn observe(nr: usize, result: isize) {
    let process_id = scheduler::current_process().lock().id();
    let mut evidence = EVIDENCE.lock();
    let Some(evidence) = evidence.as_mut() else { return };
    assert!(process_id == evidence.producer_id || process_id == evidence.consumer_id);
    if result >= 0 && nr < evidence.calls.len() { evidence.calls[nr] += 1; }
    if nr == 11 && result == -28 {
        evidence.quota_errors += 1;
        // The first rejection precedes any successful user VMO creation.
        if evidence.quota_errors == 1 { assert_eq!(vmo::user_budget_stats().1, evidence.initial_user_pages); }
    }
    if process_id == evidence.consumer_id && nr == 12 && result == 0
        && evidence.producer.lock().process_count() == 0
    {
        assert!(evidence.owner.is_closed());
        assert_eq!(vmo::user_budget_stats().1, evidence.initial_user_pages + 2);
        let retained = evidence.owner.used();
        assert!(retained >= 8192, "foreign VMO lost its creator charge");
        assert!(evidence.retained_bytes.replace(retained).is_none());
    }
    if process_id == evidence.consumer_id && nr == 2 && result == 0
        && !evidence.released
    {
        if let Some(retained) = evidence.retained_bytes {
            assert_eq!(vmo::user_budget_stats().1, evidence.initial_user_pages);
            assert!(evidence.owner.used() <= retained - 8192, "final VMO owner retained backing charge");
            evidence.released = true;
        }
    }
}

pub fn finish(status: usize, handles: usize) {
    assert_eq!(status, 0, "Job accounting program failed");
    assert_eq!(handles, 0, "Job accounting handle leak");
    let process_id = scheduler::current_process().lock().id();
    let mut evidence = EVIDENCE.lock();
    let evidence = evidence.as_mut().expect("Job accounting observer active");
    if process_id == evidence.producer_id {
        evidence.producer.lock().kill();
    } else {
        assert_eq!(process_id, evidence.consumer_id);
        assert!(evidence.released, "consumer exited before releasing foreign backing");
    }
    evidence.exits += 1;
}

fn batch(root: &Arc<SpinLock<Job>>) {
    let parent = root.lock().create_child_job("accounting_probe", 8 * 1024 * 1024)
        .expect("accounting parent Job");
    let producer_job = parent.lock().create_child_job("accounting_producer", 4 * 1024 * 1024)
        .expect("accounting producer Job");
    let consumer_job = parent.lock().create_child_job("accounting_consumer", 4 * 1024 * 1024)
        .expect("accounting consumer Job");
    let owner = producer_job.lock().budget().clone();
    let initial_user_pages = vmo::user_budget_stats().1;
    assert!(vmo::user_budget_stats().0 - initial_user_pages >= 4096,
        "accounting probe needs room for its quota-denial control");
    let (producer, consumer) = Channel::create().expect("accounting bootstrap pair");
    for (endpoint, role) in [(&consumer, 1), (&producer, 2)] {
        endpoint.send(ChannelMessage::new(alloc::vec![role], alloc::vec![]))
            .unwrap_or_else(|(error, _)| panic!("accounting role: {error:?}"));
    }
    let mut admitted = [0; 2];
    for (index, (endpoint, job)) in [(producer, &producer_job), (consumer, &consumer_job)].into_iter().enumerate() {
        let node = CAP_NODE_ALLOCATOR.alloc_root(endpoint, ObjectType::Channel,
            Rights::READ | Rights::WRITE | Rights::WAIT).expect("accounting bootstrap capability");
        admitted[index] = super::elf_load::admit_named(job,
            super::boot_payload::bytes().expect("boot payload required"), Some(node), "accounting_probe")
            .expect("load accounting userspace ELF").process_id;
    }
    *EVIDENCE.lock() = Some(Evidence {
        calls: [0; 17], exits: 0, quota_errors: 0,
        producer_id: admitted[0], consumer_id: admitted[1],
        producer: Arc::clone(&producer_job), owner,
        initial_user_pages, retained_bytes: None, released: false,
    });
    let stats = scheduler::run();
    assert_eq!(stats.reaped, 2);
    assert_eq!(scheduler::task_count(), 0);
    assert_eq!(scheduler::ready_count(), 0);
    let evidence = EVIDENCE.lock().take().expect("accounting evidence");
    assert_eq!(evidence.exits, 2);
    assert_eq!(evidence.quota_errors, 2);
    assert!(evidence.retained_bytes.is_some() && evidence.released);
    for (nr, minimum) in [(2, 4), (5, 1), (8, 3), (10, 5), (11, 1), (12, 2), (13, 1)] {
        assert!(evidence.calls[nr] >= minimum, "incomplete accounting protocol syscall {nr}");
    }
    assert_eq!(producer_job.lock().process_count(), 0);
    assert_eq!(consumer_job.lock().process_count(), 0);
    drop(evidence);
    for job in [producer_job, consumer_job] {
        let id = job.lock().id();
        job.lock().kill();
        let detached = parent.lock().detach_child_job(id);
        drop(detached);
        drop(job);
    }
    let id = parent.lock().id();
    parent.lock().kill();
    let detached = root.lock().detach_child_job(id);
    drop(detached);
    drop(parent);
}

pub fn run(root: &Arc<SpinLock<Job>>) {
    batch(root);
    let frames = bitmap::stats().free_pages;
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    let processes = root.lock().process_count();
    let jobs = root.lock().children_count();
    let root_bytes = root.lock().memory_used();
    let system_bytes = BudgetRef::system().used();
    let user_budget = vmo::user_budget_stats();
    for _ in 0..3 {
        batch(root);
        assert_eq!(bitmap::stats().free_pages, frames, "accounting frame leak");
        assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes, "accounting node leak");
        assert_eq!(root.lock().process_count(), processes, "accounting process leak");
        assert_eq!(root.lock().children_count(), jobs, "accounting Job leak");
        assert_eq!(root.lock().memory_used(), root_bytes, "accounting root ledger leak");
        assert_eq!(BudgetRef::system().used(), system_bytes, "accounting System ledger leak");
        assert_eq!(vmo::user_budget_stats(), user_budget, "accounting VMO budget leak");
    }
    crate::kprintln!("[FERBER_TEST] job_accounting: PASS");
    crate::kprintln!("[FERBER_TEST] boot_payload: PASS");
}
