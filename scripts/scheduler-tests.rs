//! Production idle classifier, source registry and channel identities.
#![feature(allocator_ext)]
#![allow(dead_code)]
extern crate alloc;
#[path = "accounting-host.rs"]
mod accounting_host;
pub use accounting_host::accounting;
#[path = "../ferber/src/kernel/scheduler/idle.rs"]
mod idle;
#[path = "../ferber/src/kernel/scheduler/sources.rs"]
mod sources;
#[path = "../ferber/src/kernel/ipc/channel.rs"]
mod channel;
#[path = "../ferber/src/kernel/sync.rs"]
pub mod sync;
mod kernel {
    pub mod memory { pub use crate::accounting; }
    pub use crate::sync;
    pub mod ipc { pub(crate) use crate::channel; }
    pub mod task {
        #[derive(Debug, PartialEq, Eq)]
        pub enum TaskError { ProcessDead, QuotaExceeded }
    }
    pub mod capabilities {
        pub trait KernelObject: Send + Sync { fn type_name(&self) -> &'static str; }
        #[derive(Debug)]
        pub struct CapNode;
    }
}
use std::sync::{Arc, Mutex};
use channel::{Channel, WaitToken};
use sources::{register_source, SourceKind};
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
    assert!(sources::snapshot().iter().flatten().any(|source| source.live() && wait.peer_of(source)));
    drop(sender);
    assert!(!wait.live());
    assert!(!wait.registered(&token, 1));
    assert!(token.is_notified(1));
    assert!(sources::snapshot().iter().flatten().all(|source| !source.live()));
    drop(receiver);
    drop(lease);
    assert_eq!(sources::count(), 0);
}

#[test]
fn bounded_sources_cancel_and_stale_generation() {
    let _serial = SERIAL.lock().unwrap();
    let (_receiver, sender) = Channel::create().unwrap();
    let mut leases: Vec<_> = (0..sources::MAX_SOURCES).map(|_| register_source(&sender, SourceKind::Software).unwrap()).collect();
    assert!(matches!(register_source(&sender, SourceKind::Software), Err(kernel::task::TaskError::QuotaExceeded)));
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
