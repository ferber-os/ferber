//! Fixed interrupt mailboxes. Publishing performs no allocation or lock acquisition.
#![allow(dead_code)]
use super::Error;
use core::sync::atomic::{AtomicU64, Ordering};
pub const MAX_BINDINGS: usize = 64;
const COUNT_MASK: u64 = 0xffff;
const MAX_GENERATION: u64 = u64::MAX >> 16;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SLOTS: [AtomicU64; MAX_BINDINGS] = [const { AtomicU64::new(0) }; MAX_BINDINGS];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    slot: usize,
    generation: u64,
}
impl Token {
    pub fn slot(self) -> usize {
        self.slot
    }
    pub fn generation(self) -> u64 {
        self.generation
    }
    /// The platform must publish this token to its IRQ routing only after allocation.
    pub fn allocate() -> Result<Self, Error> {
        let generation = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n <= MAX_GENERATION).then(|| n + 1)
            })
            .map_err(|_| Error::Quota)?;
        for (slot, state) in SLOTS.iter().enumerate() {
            if state
                .compare_exchange(0, generation << 16, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(Self { slot, generation });
            }
        }
        Err(Error::Quota)
    }
    pub fn publish(self) -> bool {
        let state = &SLOTS[self.slot];
        state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                if old >> 16 != self.generation {
                    return None;
                }
                Some((old & !COUNT_MASK) | ((old & COUNT_MASK).saturating_add(1).min(COUNT_MASK)))
            })
            .is_ok()
    }
    pub fn pending(self) -> Result<u16, Error> {
        let old = SLOTS[self.slot].load(Ordering::Acquire);
        if old >> 16 != self.generation {
            return Err(Error::Revoked);
        }
        Ok((old & COUNT_MASK) as u16)
    }
    /// Called by dispatcher/acknowledgement code, never a replacement for device EOI.
    pub fn take(self) -> Result<u16, Error> {
        SLOTS[self.slot]
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                (old >> 16 == self.generation).then_some(self.generation << 16)
            })
            .map(|old| (old & COUNT_MASK) as u16)
            .map_err(|_| Error::Revoked)
    }
    /// Mask hardware routing first. Late publications for this generation are inert.
    pub fn release(self) -> Result<(), Error> {
        SLOTS[self.slot]
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                (old >> 16 == self.generation).then_some(0)
            })
            .map(|_| ())
            .map_err(|_| Error::Revoked)
    }
}
