//! Host concurrency and allocation-failure tests for the kernel channel implementation.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;

#[path = "../wolfram/src/kernel/ipc/channel.rs"]
mod channel;
#[path = "../wolfram/src/kernel/sync.rs"]
pub mod sync;

// Only capability ownership is needed here; in-kernel tests use real CapNodes.
mod kernel {
    pub use crate::sync;
    pub mod capabilities {
        pub trait KernelObject: Send + Sync {
            fn type_name(&self) -> &'static str;
        }
        #[derive(Debug)]
        pub struct CapNode;
    }
}

use channel::{Channel, ChannelError, ChannelMessage, SIGNAL_PEER_CLOSED, SIGNAL_WRITABLE};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

thread_local! {
    static FAIL_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
}

struct TestAllocator;
unsafe impl GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let fail = FAIL_AFTER
            .try_with(|remaining| match remaining.get() {
                Some(0) => true,
                Some(count) => {
                    remaining.set(Some(count - 1));
                    false
                }
                None => false,
            })
            .unwrap_or(false);
        if fail {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: TestAllocator = TestAllocator;

#[test]
fn opposite_direction_sends_complete_without_lock_inversion() {
    let (first, second) = Channel::create_with_limits(512, 4, 0).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let (done, result) = mpsc::channel();
    for endpoint in [first, second] {
        let barrier = Arc::clone(&barrier);
        let done = done.clone();
        std::thread::spawn(move || {
            barrier.wait();
            for index in 0..512u32 {
                endpoint
                    .send(ChannelMessage::new(
                        index.to_le_bytes().to_vec(),
                        Vec::new(),
                    ))
                    .unwrap();
                loop {
                    match endpoint.recv() {
                        Ok(message) => {
                            assert_eq!(message.bytes, index.to_le_bytes());
                            break;
                        }
                        Err(ChannelError::QueueEmpty) => std::thread::yield_now(),
                        error => panic!("unexpected receive: {error:?}"),
                    }
                }
            }
            done.send(()).unwrap();
        });
    }
    barrier.wait();
    for _ in 0..2 {
        result
            .recv_timeout(Duration::from_secs(10))
            .expect("opposite sends deadlocked");
    }
}

#[test]
fn allocation_failures_preserve_admission_and_messages() {
    for allocation in 0..3 {
        FAIL_AFTER.set(Some(allocation));
        let result = Channel::create();
        FAIL_AFTER.set(None);
        assert!(matches!(result, Err(ChannelError::OutOfMemory)));
    }
    let (first, second) = Channel::create().unwrap();
    let message = ChannelMessage::new(vec![3], Vec::new());
    FAIL_AFTER.set(Some(0));
    let result = first.send(message);
    FAIL_AFTER.set(None);
    let (error, returned) = result.unwrap_err();
    assert_eq!(error, ChannelError::OutOfMemory);
    assert_eq!(returned.bytes, [3]);
    assert_eq!(second.queue_len(), 0);
    first.send(returned).unwrap();
    assert_eq!(second.recv().unwrap().bytes, [3]);
}

#[test]
fn failed_receive_retries_and_closed_peer_drains() {
    let (first, second) = Channel::create_with_limits(1, 4, 0).unwrap();
    first
        .send(ChannelMessage::new(vec![7], Vec::new()))
        .unwrap();
    assert_eq!(first.signals() & SIGNAL_WRITABLE, 0);
    let rejected = first.send_with::<ChannelError>(0, 0, || panic!("full queue ran commit"));
    assert_eq!(rejected, Err(ChannelError::QueueFull));
    let rejected = second.recv_with::<(), ChannelError>(|_| Err(ChannelError::MessageTooLarge));
    assert_eq!(rejected.unwrap_err(), ChannelError::MessageTooLarge);
    drop(first);
    assert_eq!(second.recv().unwrap().bytes, [7]);
    assert_eq!(second.recv().unwrap_err(), ChannelError::PeerClosed);
    assert_eq!(second.signals(), SIGNAL_PEER_CLOSED);
}
