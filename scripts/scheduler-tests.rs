//! Production idle classifier, source registry and channel identities.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;
#[path = "accounting-host.rs"]
mod accounting_host;
pub use accounting_host::accounting;
#[path = "../ferber/src/kernel/ipc/channel.rs"]
mod channel;
#[path = "../ferber/src/kernel/scheduler/idle.rs"]
mod idle;
#[path = "../ferber/src/kernel/scheduler/sources.rs"]
mod sources;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;
mod kernel {
    pub mod memory {
        pub use crate::accounting;
    }
    pub use crate::sync;
    pub mod ipc {
        pub(crate) use crate::channel;
    }
    pub mod task {
        #[derive(Debug, PartialEq, Eq)]
        pub enum TaskError {
            ProcessDead,
            QuotaExceeded,
        }
    }
    pub mod capabilities {
        pub trait KernelObject: Send + Sync {
            fn type_name(&self) -> &'static str;
        }
        #[derive(Debug)]
        pub struct CapNode;
    }
}
use channel::{Channel, WaitToken};
use sources::{register_source, SourceKind};
use std::sync::{Arc, Mutex};
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn producer_identity_registration_and_weak_lifetime() {
    let _serial = SERIAL.lock().unwrap();
    let (receiver, sender) = Channel::create().unwrap();
    let wait = receiver.wait_registration();
    assert!(!wait.peer_of(&receiver.wait_registration()));
    assert!(wait.peer_of(&sender.wait_registration()));
    let token = Arc::new(WaitToken::new());
    token.start(1);
    assert!(receiver.register_wait(&token, 1).unwrap());
    assert!(wait.registered(&token, 1));
    assert!(!wait.registered(&token, 2));
    let lease = register_source(&sender, SourceKind::TerminalBootstrap).unwrap();
    assert_eq!(sources::count(), 1);
    assert!(sources::snapshot()
        .iter()
        .flatten()
        .any(|source| source.live() && wait.peer_of(source)));
    drop(sender);
    assert!(!wait.live());
    assert!(!wait.registered(&token, 1));
    assert!(token.is_notified(1));
    assert!(sources::snapshot()
        .iter()
        .flatten()
        .all(|source| !source.live()));
    drop(receiver);
    drop(lease);
    assert_eq!(sources::count(), 0);
}

#[test]
fn bounded_sources_cancel_and_stale_generation() {
    let _serial = SERIAL.lock().unwrap();
    let (_receiver, sender) = Channel::create().unwrap();
    let mut leases: Vec<_> = (0..sources::MAX_SOURCES)
        .map(|_| register_source(&sender, SourceKind::Software).unwrap())
        .collect();
    assert!(matches!(
        register_source(&sender, SourceKind::Software),
        Err(kernel::task::TaskError::QuotaExceeded)
    ));
    let mut old = leases.remove(0);
    old.cancel();
    let replacement = register_source(&sender, SourceKind::Software).unwrap();
    assert_eq!(sources::count(), sources::MAX_SOURCES);
    old.cancel();
    drop(old);
    assert_eq!(sources::count(), sources::MAX_SOURCES);
    drop(replacement);
    drop(leases);
    assert_eq!(sources::count(), 0);
}

#[test]
fn shared_wait_generation_notifies_from_any_endpoint_and_cancels_all() {
    let _serial = SERIAL.lock().unwrap();
    let pairs: Vec<_> = (0..16).map(|_| Channel::create().unwrap()).collect();
    let token = Arc::new(WaitToken::new());
    token.start(74);
    for (receiver, _) in &pairs {
        assert!(receiver.register_wait(&token, 74).unwrap());
    }
    pairs[15]
        .1
        .send(channel::ChannelMessage::new(Vec::new(), Vec::new()))
        .unwrap();
    assert!(token.is_notified(74));
    assert!(pairs[15].0.readable());
    for (receiver, _) in &pairs {
        receiver.unregister_wait(&token, 74);
    }
    token.start(75);
    token.notify(74);
    assert!(!token.is_notified(75));
    pairs[15].0.recv().unwrap();
    for (receiver, _) in &pairs {
        assert!(!receiver.wait_registration().registered(&token, 74));
        assert!(receiver.register_wait(&token, 75).unwrap());
    }
    // A notification between registrations stays latched before the caller parks.
    pairs[0]
        .1
        .send(channel::ChannelMessage::new(Vec::new(), Vec::new()))
        .unwrap();
    assert!(token.is_notified(75));
    for (receiver, _) in &pairs {
        receiver.unregister_wait(&token, 75);
    }
}

#[test]
fn closed_peer_is_readable_without_consuming_queued_messages() {
    let _serial = SERIAL.lock().unwrap();
    let (receiver, sender) = Channel::create().unwrap();
    assert!(!receiver.readable());
    sender
        .send(channel::ChannelMessage::new(vec![74], Vec::new()))
        .unwrap();
    assert!(receiver.readable());
    assert!(receiver.readable());
    assert_eq!(receiver.recv().unwrap().bytes, [74]);
    assert!(!receiver.readable());
    drop(sender);
    assert!(receiver.readable());
    let token = Arc::new(WaitToken::new());
    token.start(1);
    assert!(!receiver.register_wait(&token, 1).unwrap());
}
