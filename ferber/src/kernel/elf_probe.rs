//! Checkpoint A: independently linked bytes, kernel-constructed task lifecycle.

use super::capabilities::CAP_NODE_ALLOCATOR;
use super::memory::bitmap;
use super::scheduler;
use super::sync::SpinLock;
use super::task::Job;
use crate::kprintln;
use alloc::sync::Arc;

// shortcut: embedded test image; replace with a reserved boot payload for production startup.
static IMAGE: &[u8] = include_bytes!(env!("FERBER_USER_ELF"));
static STAGE: SpinLock<Option<usize>> = SpinLock::new(None);

pub fn observe(nr: usize, result: isize) {
    let mut stage = STAGE.lock();
    let Some(value) = stage.as_mut() else { return };
    match *value {
        0 => {
            assert_eq!(nr, 0x7fff);
            assert_eq!(result, -38);
        }
        1 => {
            assert_eq!(nr, super::syscall::SYS_YIELD);
            assert_eq!(result, 0);
        }
        _ => panic!("unexpected ELF syscall"),
    }
    *value += 1;
}

pub fn finish(status: usize, handles: usize) {
    if let Some(stage) = STAGE.lock().as_mut() {
        assert_eq!(*stage, 2, "incomplete ELF protocol");
        assert_eq!(status, 0, "ELF program failed");
        assert_eq!(handles, 0, "ELF handle leak");
        *stage = 3;
    }
}

fn batch(root: &Arc<SpinLock<Job>>) {
    *STAGE.lock() = Some(0);
    super::elf_load::admit(root, IMAGE).expect("load userspace ELF");
    let stats = scheduler::run();
    assert_eq!(STAGE.lock().take(), Some(3), "ELF did not exit");
    assert_eq!(stats.yields, 1);
    assert_eq!(stats.reaped, 1);
    assert_eq!(scheduler::task_count(), 0);
    assert_eq!(scheduler::ready_count(), 0);
}

pub fn run(root: &Arc<SpinLock<Job>>) {
    // Warm allocator and scheduler capacities before exact reclamation checks.
    batch(root);
    let frames = bitmap::stats().free_pages;
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    let processes = root.lock().process_count();
    for _ in 0..3 {
        batch(root);
        assert_eq!(bitmap::stats().free_pages, frames, "ELF frame leak");
        assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes, "ELF node leak");
        assert_eq!(root.lock().process_count(), processes, "ELF process leak");
    }
    kprintln!("[FERBER_TEST] userspace_elf: PASS");
}
