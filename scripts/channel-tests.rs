//! Host concurrency and allocation-failure tests for the kernel channel implementation.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;
#[path = "accounting-host.rs"]
mod accounting_host;
pub use accounting_host::accounting;

#[path = "../ferber/src/kernel/ipc/channel.rs"]
mod channel;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;

// Only capability ownership is needed here; in-kernel tests use real CapNodes.
mod kernel {
    pub mod memory { pub use crate::accounting; }
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
use std::cell::{Cell, RefCell};
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

thread_local! {
    static FAIL_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
    static EXPECT_OWNER: RefCell<Option<accounting::BudgetRef>> = const { RefCell::new(None) };
    static OWNER_CHECKS: Cell<usize> = const { Cell::new(0) };
    static WRONG_OWNER: Cell<usize> = const { Cell::new(0) };
}

struct TestAllocator;
unsafe impl GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = EXPECT_OWNER.try_with(|expected| {
            if let Some(owner) = expected.borrow().as_ref() {
                OWNER_CHECKS.with(|count| count.set(count.get() + 1));
                if !owner.same_owner(&accounting::current()) {
                    WRONG_OWNER.with(|count| count.set(count.get() + 1));
                }
            }
        });
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

#[test]
fn receive_wait_wakes_all_and_old_generations_are_inert() {
    use channel::WaitToken;
    let (sender, receiver) = Channel::create().unwrap();
    let first = Arc::new(WaitToken::new());
    let second = Arc::new(WaitToken::new());
    first.start(1);
    second.start(1);
    assert!(receiver.register_wait(&first, 1).unwrap());
    assert!(receiver.register_wait(&first, 1).unwrap());
    assert!(receiver.register_wait(&second, 1).unwrap());
    first.start(2);
    assert!(receiver.register_wait(&first, 2).unwrap());
    sender
        .send(ChannelMessage::new(vec![9], Vec::new()))
        .unwrap();
    assert!(first.is_notified(2));
    assert!(second.is_notified(1));
    first.notify(1);
    assert!(first.is_notified(2));
    assert!(!receiver.register_wait(&first, 2).unwrap());
    assert_eq!(receiver.recv().unwrap().bytes, [9]);
    first.start(3);
    first.notify(2);
    assert!(!first.is_notified(3));
    assert!(receiver.register_wait(&first, 3).unwrap());
    drop(sender);
    assert!(first.is_notified(3));
    assert!(!receiver.register_wait(&first, 3).unwrap());
    assert_eq!(receiver.recv().unwrap_err(), ChannelError::PeerClosed);
}

#[test]
fn send_registration_race_never_loses_readiness() {
    use channel::WaitToken;
    for _ in 0..128 {
        let (sender, receiver) = Channel::create().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let gate = Arc::clone(&barrier);
        let sender_thread = std::thread::spawn(move || {
            gate.wait();
            sender
                .send(ChannelMessage::new(Vec::new(), Vec::new()))
                .unwrap();
            sender
        });
        let token = Arc::new(WaitToken::new());
        token.start(1);
        barrier.wait();
        let registered = receiver.register_wait(&token, 1).unwrap();
        let sender = sender_thread.join().unwrap();
        assert!(!registered || token.is_notified(1));
        assert_eq!(receiver.recv().unwrap().bytes.len(), 0);
        drop(sender);
    }
}

#[test]
fn wait_storage_failure_is_transactional_and_tickets_do_not_retain_tasks() {
    use channel::WaitToken;
    let (sender, receiver) = Channel::create().unwrap();
    let token = Arc::new(WaitToken::new());
    token.start(1);
    FAIL_AFTER.set(Some(0));
    let result = receiver.register_wait(&token, 1);
    FAIL_AFTER.set(None);
    assert_eq!(result, Err(ChannelError::OutOfMemory));
    assert!(!token.is_notified(1));
    assert_eq!(receiver.queue_len(), 0);
    assert!(receiver.register_wait(&token, 1).unwrap());
    assert_eq!(Arc::strong_count(&token), 1);
    let weak = Arc::downgrade(&token);
    drop(token);
    assert!(weak.upgrade().is_none());
    sender
        .send(ChannelMessage::new(Vec::new(), Vec::new()))
        .unwrap();
    assert!(receiver.recv().is_ok());
}

#[test]
fn failed_send_does_not_notify_and_wake_needs_no_allocation() {
    use channel::WaitToken;
    let (sender, receiver) = Channel::create_with_limits(1, 1, 0).unwrap();
    sender
        .send(ChannelMessage::new(vec![1], Vec::new()))
        .unwrap();
    receiver.recv().unwrap();
    let token = Arc::new(WaitToken::new());
    token.start(1);
    assert!(receiver.register_wait(&token, 1).unwrap());
    let rejected = sender.send(ChannelMessage::new(vec![1, 2], Vec::new()));
    assert_eq!(rejected.unwrap_err().0, ChannelError::MessageTooLarge);
    assert!(!token.is_notified(1));
    FAIL_AFTER.set(Some(0));
    let result = sender.send(ChannelMessage::new(Vec::new(), Vec::new()));
    FAIL_AFTER.set(None);
    assert!(result.is_ok());
    assert!(token.is_notified(1));
}

#[test]
fn closure_registration_race_never_loses_closed_state() {
    use channel::WaitToken;
    for _ in 0..128 {
        let (sender, receiver) = Channel::create().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let gate = Arc::clone(&barrier);
        let closer = std::thread::spawn(move || {
            gate.wait();
            drop(sender);
        });
        let token = Arc::new(WaitToken::new());
        token.start(1);
        barrier.wait();
        let registered = receiver.register_wait(&token, 1).unwrap();
        closer.join().unwrap();
        assert!(!registered || token.is_notified(1));
        assert_eq!(receiver.recv().unwrap_err(), ChannelError::PeerClosed);
    }
}

#[test]
fn canceled_registration_is_removed_without_canceling_new_generation() {
    use channel::WaitToken;
    let (sender, receiver) = Channel::create().unwrap();
    let token = Arc::new(WaitToken::new());
    token.start(1);
    assert!(receiver.register_wait(&token, 1).unwrap());
    receiver.unregister_wait(&token, 1);
    assert_eq!(Arc::weak_count(&token), 0);
    sender
        .send(ChannelMessage::new(Vec::new(), Vec::new()))
        .unwrap();
    assert!(!token.is_notified(1));
    receiver.recv().unwrap();
    token.start(2);
    assert!(receiver.register_wait(&token, 2).unwrap());
    receiver.unregister_wait(&token, 1);
    drop(sender);
    assert!(token.is_notified(2));
    assert!(!token.is_notified(0));
}

#[test]
fn registration_cancel_survives_original_owner_without_retaining_pair() {
    use channel::WaitToken;
    let (sender, receiver) = Channel::create().unwrap();
    let alias = Arc::clone(&receiver);
    let token = Arc::new(WaitToken::new());
    token.start(1);
    let registration = receiver.wait_registration();
    assert_eq!(Arc::strong_count(&receiver), 2);
    assert!(receiver.register_wait(&token, 1).unwrap());
    drop(receiver);
    registration.unregister(&token, 1);
    assert_eq!(Arc::weak_count(&token), 0);
    assert!(!token.is_notified(1));
    assert!(alias.register_wait(&token, 1).unwrap());
    drop(alias);
    assert!(token.is_notified(1));
    assert_eq!(Arc::weak_count(&token), 0);
    drop(sender);
    registration.unregister(&token, 1);
    drop(registration);
}

#[test]
fn failed_send_preserves_registration_and_teardown_needs_no_allocation() {
    use channel::WaitToken;
    let (sender, receiver) = Channel::create().unwrap();
    let token = Arc::new(WaitToken::new());
    token.start(1);
    assert!(receiver.register_wait(&token, 1).unwrap());
    let message = ChannelMessage::new(vec![74], Vec::new());
    FAIL_AFTER.set(Some(0));
    let result = sender.send(message);
    FAIL_AFTER.set(None);
    let (error, returned) = result.unwrap_err();
    assert_eq!(error, ChannelError::OutOfMemory);
    assert_eq!(returned.bytes, [74]);
    assert!(!token.is_notified(1));
    assert_eq!(Arc::weak_count(&token), 1);
    assert_eq!(receiver.queue_len(), 0);
    sender.send(returned).unwrap();
    assert!(token.is_notified(1));
    assert_eq!(receiver.recv().unwrap().bytes, [74]);
    token.start(2);
    assert!(receiver.register_wait(&token, 2).unwrap());
    let registration = receiver.wait_registration();
    FAIL_AFTER.set(Some(0));
    drop(sender);
    assert!(token.is_notified(2));
    registration.unregister(&token, 2);
    drop(receiver);
    registration.unregister(&token, 2);
    drop(registration);
    FAIL_AFTER.set(None);
    assert_eq!(Arc::weak_count(&token), 0);
}

#[test]
fn repeated_wait_generations_release_all_weak_registrations() {
    use channel::WaitToken;
    let (sender, receiver) = Channel::create().unwrap();
    let token = Arc::new(WaitToken::new());
    for generation in 1..2048 {
        token.start(generation);
        assert!(receiver.register_wait(&token, generation).unwrap());
        receiver.unregister_wait(&token, generation - 1);
        assert_eq!(Arc::weak_count(&token), 1);
        receiver.unregister_wait(&token, generation);
        assert_eq!(Arc::weak_count(&token), 0);
        token.notify(generation - 1);
        assert!(!token.is_notified(generation));
    }
    token.start(2048);
    assert!(receiver.register_wait(&token, 2048).unwrap());
    drop(sender);
    assert!(token.is_notified(2048));
    assert_eq!(Arc::weak_count(&token), 0);
}

#[test]
fn foreign_queue_and_waiter_growth_uses_container_owner() {
    let owner = accounting::JobBudget::child(&accounting::BudgetRef::system(), 1 << 20).unwrap();
    let foreign = accounting::JobBudget::child(&accounting::BudgetRef::system(), 1 << 20).unwrap();
    let (sender, receiver) = accounting::with_budget(&owner, || Channel::create().unwrap());
    let message = accounting::with_budget(&foreign, || ChannelMessage::new(vec![0x74], vec![]));
    EXPECT_OWNER.with(|slot| *slot.borrow_mut() = Some(owner.clone()));
    OWNER_CHECKS.set(0);
    WRONG_OWNER.set(0);
    accounting::with_budget(&foreign, || sender.send(message).unwrap());
    EXPECT_OWNER.with(|slot| *slot.borrow_mut() = None);
    assert!(OWNER_CHECKS.get() > 0);
    assert_eq!(WRONG_OWNER.get(), 0);
    drop(receiver.recv().unwrap());
    let token = Arc::new(channel::WaitToken::new());
    token.start(1);
    EXPECT_OWNER.with(|slot| *slot.borrow_mut() = Some(owner));
    OWNER_CHECKS.set(0);
    WRONG_OWNER.set(0);
    assert!(accounting::with_budget(&foreign, || receiver.register_wait(&token, 1).unwrap()));
    EXPECT_OWNER.with(|slot| *slot.borrow_mut() = None);
    assert!(OWNER_CHECKS.get() > 0);
    assert_eq!(WRONG_OWNER.get(), 0);
    receiver.unregister_wait(&token, 1);
}
