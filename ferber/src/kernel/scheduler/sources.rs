//! Kernel-owned producer leases do not retain channel endpoints.
#![cfg_attr(not(feature = "init"), allow(dead_code))]

use alloc::sync::Arc;
use crate::kernel::ipc::channel::{Channel, WaitRegistration};
use crate::kernel::sync::SpinLock;
use crate::kernel::task::TaskError;

pub const MAX_SOURCES: usize = 64;

#[derive(Clone, Copy)]
pub enum SourceKind {
    /// This release may diagnose anchored quiescence and halt; no IRQ wake is promised.
    TerminalBootstrap,
    /// The owner must cancel the lease when its concrete pending work is drained.
    Software,
}

struct Source {
    generation: u64,
    channel: WaitRegistration,
    kind: SourceKind,
}
struct Registry {
    sources: [Option<Source>; MAX_SOURCES],
    next: u64,
}
static SOURCES: SpinLock<Registry> = SpinLock::new(Registry {
    sources: [const { None }; MAX_SOURCES], next: 1,
});

pub struct SourceLease {
    slot: usize,
    generation: u64,
    active: bool,
}

pub fn register_source(channel: &Arc<Channel>, kind: SourceKind) -> Result<SourceLease, TaskError> {
    let channel = channel.wait_registration();
    if !channel.live() { return Err(TaskError::ProcessDead); }
    let mut sources = SOURCES.lock();
    let slot = sources.sources.iter().position(Option::is_none).ok_or(TaskError::QuotaExceeded)?;
    let generation = sources.next;
    sources.next = generation.checked_add(1).ok_or(TaskError::QuotaExceeded)?;
    sources.sources[slot] = Some(Source { generation, channel, kind });
    Ok(SourceLease { slot, generation, active: true })
}

impl SourceLease {
    pub fn cancel(&mut self) {
        if !self.active { return; }
        let removed = {
            let mut sources = SOURCES.lock();
            if sources.sources[self.slot].as_ref().is_some_and(|source| source.generation == self.generation) {
                sources.sources[self.slot].take()
            } else { None }
        };
        self.active = false;
        drop(removed);
    }
}
impl Drop for SourceLease {
    fn drop(&mut self) { self.cancel(); }
}

pub(super) fn snapshot() -> [Option<WaitRegistration>; MAX_SOURCES] {
    let sources = SOURCES.lock();
    core::array::from_fn(|index| sources.sources[index].as_ref().map(|source| {
        match source.kind { SourceKind::TerminalBootstrap | SourceKind::Software => {} }
        source.channel.clone()
    }))
}

#[cfg(test)]
pub(super) fn count() -> usize {
    SOURCES.lock().sources.iter().flatten().count()
}
