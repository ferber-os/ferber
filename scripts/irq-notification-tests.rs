//! Production IRQ registry, source leases and channels under bounded failure scenarios.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;
#[path = "accounting-host.rs"]
mod accounting_host;
pub use accounting_host::accounting;
#[path = "../ferber/src/kernel/ipc/channel.rs"]
mod channel;
#[path = "../ferber/src/kernel/device/irq.rs"]
pub mod irq;
#[path = "../ferber/src/kernel/device/notifications.rs"]
mod notifications;
#[path = "../ferber/src/kernel/scheduler/sources.rs"]
mod sources;
#[path = "../ferber/src/kernel/device/state.rs"]
mod state;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;
pub use state::Error;
mod kernel {
    pub mod memory {
        pub use crate::accounting;
    }
    pub use crate::sync;
    pub mod ipc {
        pub(crate) use crate::channel;
    }
    pub mod scheduler {
        pub use crate::sources::{register_source, SourceKind, SourceLease};
    }
    pub mod task {
        #[derive(Debug, PartialEq, Eq)]
        pub enum TaskError {
            ProcessDead,
            QuotaExceeded,
        }
        #[derive(Debug, PartialEq, Eq)]
        pub enum ProcessState {
            Running,
            Dead,
        }
        pub struct Process(pub ProcessState);
        impl Process {
            pub fn budget(&self) -> &crate::accounting::BudgetRef {
                &crate::accounting::BudgetRef::System
            }
            pub fn state(&self) -> ProcessState {
                match self.0 {
                    ProcessState::Running => ProcessState::Running,
                    ProcessState::Dead => ProcessState::Dead,
                }
            }
        }
    }
    pub mod capabilities {
        use core::sync::atomic::{AtomicBool, Ordering};
        pub trait KernelObject: Send + Sync {
            fn type_name(&self) -> &'static str;
        }
        #[derive(Debug)]
        pub struct CapNode(pub AtomicBool);
        impl CapNode {
            pub fn is_valid(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
        }
    }
}
use channel::Channel;
use kernel::{
    capabilities::CapNode,
    task::{Process, ProcessState},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use sync::SpinLock;
static SERIAL: Mutex<()> = Mutex::new(());
fn fixture(
    depth: usize,
) -> (
    Arc<CapNode>,
    Arc<SpinLock<Process>>,
    Arc<Channel>,
    irq::Token,
) {
    let node = Arc::new(CapNode(AtomicBool::new(true)));
    let owner = Arc::new(SpinLock::new(Process(ProcessState::Running)));
    let (sender, receiver) = Channel::create_with_limits(depth, 32, 0).unwrap();
    let token = notifications::bind(Arc::downgrade(&node), Arc::downgrade(&owner), sender).unwrap();
    (node, owner, receiver, token)
}
fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
#[test]
fn only_an_armed_route_roots_idle_and_ack_sequence_is_single_use() {
    let _serial = SERIAL.lock().unwrap();
    let (_node, _owner, receiver, token) = fixture(2);
    assert_eq!(sources::count(), 0);
    unsafe { notifications::activate(token).unwrap() };
    assert_eq!(sources::count(), 1);
    assert!(token.publish());
    notifications::drain();
    assert_eq!(sources::count(), 0);
    let message = receiver.recv().unwrap();
    assert_eq!(&message.bytes[..8], b"FRIRQ001");
    assert_eq!(word(&message.bytes, 8), token.generation());
    assert_eq!(word(&message.bytes, 16), 1);
    assert_eq!(
        u32::from_le_bytes(message.bytes[24..28].try_into().unwrap()),
        1
    );
    assert!(message.handles.is_empty());
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 0),
        Err(Error::Invalid)
    );
    assert_eq!(unsafe { notifications::activate(token) }, Err(Error::Busy));
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 1),
        Ok(true)
    );
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 1),
        Err(Error::Invalid)
    );
    unsafe {
        notifications::activate(token).unwrap();
        notifications::release(token).unwrap()
    };
    assert_eq!(sources::count(), 0);
}
#[test]
fn full_queue_preserves_count_and_does_not_consume_sequence() {
    let _serial = SERIAL.lock().unwrap();
    let (node, owner, receiver, token) = fixture(1);
    // Fill the receiver through a separate binding to the same sender.
    // The fixed registry's retained endpoint is exercised by sending one real IRQ first.
    unsafe { notifications::activate(token).unwrap() };
    token.publish();
    notifications::drain();
    token.publish();
    token.publish();
    notifications::drain();
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 1),
        Ok(false)
    );
    notifications::drain(); // Receiver queue still contains sequence 1.
    assert_eq!(word(&receiver.recv().unwrap().bytes, 16), 1);
    notifications::drain();
    let second = receiver.recv().unwrap();
    assert_eq!(word(&second.bytes, 16), 2);
    assert_eq!(
        u32::from_le_bytes(second.bytes[24..28].try_into().unwrap()),
        2
    );
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 1),
        Err(Error::Invalid)
    );
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 2),
        Ok(true)
    );
    unsafe { notifications::release(token).unwrap() };
    drop((node, owner));
}
#[test]
fn invalid_authority_cancels_source_without_releasing_hardware_token() {
    let _serial = SERIAL.lock().unwrap();
    let (node, _owner, receiver, token) = fixture(1);
    unsafe { notifications::activate(token).unwrap() };
    node.0.store(false, Ordering::Release);
    token.publish();
    notifications::drain();
    assert_eq!(sources::count(), 0);
    assert!(receiver.recv().is_err());
    assert_eq!(token.pending(), Ok(1));
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 1),
        Err(Error::Revoked)
    );
    unsafe { notifications::release(token).unwrap() };
    assert!(!token.publish());
}
#[test]
fn dead_owner_and_released_generation_cannot_rearm() {
    let _serial = SERIAL.lock().unwrap();
    let (_node, owner, _receiver, token) = fixture(1);
    owner.lock().0 = ProcessState::Dead;
    assert_eq!(
        unsafe { notifications::activate(token) },
        Err(Error::Revoked)
    );
    notifications::drain();
    unsafe { notifications::release(token).unwrap() };
    assert_eq!(
        unsafe { notifications::activate(token) },
        Err(Error::Revoked)
    );
    assert_eq!(
        notifications::acknowledge(token, token.generation(), 1),
        Err(Error::Revoked)
    );
    assert_eq!(sources::count(), 0);
}

thread_local! { static FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }
struct TestAllocator;
unsafe impl std::alloc::GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let fail = FAIL_AFTER
            .try_with(|remaining| match remaining.get() {
                Some(0) => true,
                Some(n) => {
                    remaining.set(Some(n - 1));
                    false
                }
                None => false,
            })
            .unwrap_or(false);
        if fail {
            std::ptr::null_mut()
        } else {
            unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
        }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: TestAllocator = TestAllocator;
#[test]
fn allocation_failures_preserve_event_and_delivery_sequence() {
    let _serial = SERIAL.lock().unwrap();
    for available in 0..2 {
        let (_node, _owner, receiver, token) = fixture(1);
        unsafe { notifications::activate(token).unwrap() };
        token.publish();
        FAIL_AFTER.with(|remaining| remaining.set(Some(available)));
        notifications::drain();
        FAIL_AFTER.with(|remaining| remaining.set(None));
        assert!(receiver.recv().is_err());
        assert_eq!(
            notifications::acknowledge(token, token.generation(), 1),
            Err(Error::Invalid)
        );
        notifications::drain();
        let message = receiver.recv().unwrap();
        assert_eq!(word(&message.bytes, 16), 1);
        assert_eq!(
            u32::from_le_bytes(message.bytes[24..28].try_into().unwrap()),
            1
        );
        assert_eq!(
            notifications::acknowledge(token, token.generation(), 1),
            Ok(true)
        );
        unsafe { notifications::release(token).unwrap() };
    }
}

#[test]
fn old_binding_generation_cannot_ack_new_sequence_one() {
    let _serial = SERIAL.lock().unwrap();
    let (_old_node, _old_owner, old_receiver, old) = fixture(1);
    unsafe { notifications::activate(old).unwrap() };
    old.publish();
    notifications::drain();
    let old_message = old_receiver.recv().unwrap();
    unsafe { notifications::release(old).unwrap() };
    let (_node, _owner, receiver, token) = fixture(1);
    assert_ne!(old.generation(), token.generation());
    unsafe { notifications::activate(token).unwrap() };
    token.publish();
    notifications::drain();
    assert_eq!(
        notifications::acknowledge(token, word(&old_message.bytes, 8), 1),
        Err(Error::Invalid)
    );
    assert_eq!(unsafe { notifications::activate(token) }, Err(Error::Busy));
    let message = receiver.recv().unwrap();
    assert_eq!(
        notifications::acknowledge(token, word(&message.bytes, 8), word(&message.bytes, 16)),
        Ok(true)
    );
    unsafe { notifications::release(token).unwrap() };
}
