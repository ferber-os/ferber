//! Independently linked Rust startup, explicit channel delegation and task reclamation.

use super::capabilities::{ObjectType, Rights, CAP_NODE_ALLOCATOR};
use super::ipc::channel::{Channel, ChannelMessage};
use super::memory::bitmap;
use super::scheduler;
use super::sync::SpinLock;
use super::task::Job;
use crate::kprintln;
use alloc::sync::Arc;

// shortcut: embedded test image; replace with a reserved boot payload for production startup.
static IMAGE: &[u8] = include_bytes!(env!("FERBER_USER_ELF"));
static STAGE: SpinLock<Option<usize>> = SpinLock::new(None);
const CHALLENGE: &[u8; 8] = b"ferber:1";
const REPLY: &[u8; 8] = b"ready-v1";

pub fn observe(nr: usize, result: isize) {
    let mut stage = STAGE.lock();
    let Some(value) = stage.as_mut() else { return };
    match *value {
        0 => {
            assert_eq!(nr, 0x7fff);
            assert_eq!(result, -38);
        }
        1 => {
            assert_eq!(nr, super::syscall::SYS_CHAN_RECV);
            assert_eq!(result, 0);
        }
        2 => {
            assert_eq!(nr, super::syscall::SYS_CHAN_SEND);
            assert_eq!(result, 0);
        }
        3 => {
            assert_eq!(nr, super::syscall::SYS_HANDLE_CLOSE);
            assert_eq!(result, 0);
        }
        4 => {
            assert_eq!(nr, super::syscall::SYS_YIELD);
            assert_eq!(result, 0);
        }
        _ => panic!("unexpected ELF syscall"),
    }
    *value += 1;
}

pub fn finish(status: usize, handles: usize) {
    if let Some(stage) = STAGE.lock().as_mut() {
        assert_eq!(*stage, 5, "incomplete ELF protocol");
        assert_eq!(status, 0, "ELF program failed");
        assert_eq!(handles, 0, "ELF handle leak");
        *stage = 6;
    }
}

fn batch(root: &Arc<SpinLock<Job>>) {
    let (user_endpoint, kernel_endpoint) = Channel::create().expect("ELF bootstrap channel");
    kernel_endpoint
        .send(ChannelMessage::new(
            alloc::vec::Vec::from(&CHALLENGE[..]),
            alloc::vec::Vec::new(),
        ))
        .unwrap_or_else(|(error, _)| panic!("queue ELF bootstrap challenge: {error:?}"));
    let user_node = CAP_NODE_ALLOCATOR
        .alloc_root(
            user_endpoint,
            ObjectType::Channel,
            Rights::READ | Rights::WRITE,
        )
        .expect("ELF bootstrap capability");
    *STAGE.lock() = Some(0);
    super::elf_load::admit(root, IMAGE, Some(user_node)).expect("load userspace ELF");
    let stats = scheduler::run();
    assert_eq!(STAGE.lock().take(), Some(6), "ELF did not exit");
    assert_eq!(stats.yields, 1);
    assert_eq!(stats.reaped, 1);
    assert_eq!(scheduler::task_count(), 0);
    assert_eq!(scheduler::ready_count(), 0);
    let response = kernel_endpoint.recv().expect("ELF bootstrap reply");
    assert_eq!(response.bytes.as_slice(), REPLY);
    assert!(response.handles.is_empty(), "unexpected bootstrap handles");
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
    kprintln!("[FERBER_TEST] userspace_bootstrap_channel: PASS");
}
