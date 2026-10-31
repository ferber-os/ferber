//! Retained DMA ownership for a verified hardware assignment.
#![allow(dead_code)]
extern crate alloc;
use crate::kernel::capabilities::Rights;
use crate::kernel::memory::{
    paging::MapFlags,
    vmo::{Vmo, WritableMapping},
};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
#[path = "../../../../shared/device_abi.rs"]
pub mod abi;
pub mod irq;
pub mod region;
pub mod state;
use state::Lifecycle;
pub use state::{Error, State};
pub static QUARANTINED_ASSIGNMENTS: AtomicUsize = AtomicUsize::new(0);
pub static QUARANTINED_DMA_BYTES: AtomicUsize = AtomicUsize::new(0);
/// Trusted platform operations, never implemented by a userspace driver.
///
/// # Safety
/// `isolate` must establish deny-by-default DMA/interrupt routing with bus mastering
/// disabled. `install` publishes only the supplied owned frames at the given IOVA.
/// `remove` must synchronously invalidate all translations before returning success.
/// `start` must not enable hardware until its protected domain and IRQ routing exist.
/// `quiesce` success proves bus mastering and IRQ delivery disabled, the controller
/// reset/stopped and all DMA/interrupt activity drained before frames can be reused.
/// Every uncertain hardware outcome must return an error. No method may allocate
/// or release the driver's backing independently of the retained mapping leases.
pub unsafe trait Backend: Send + Sync {
    fn isolate(&mut self) -> Result<(), Error>;
    fn install(
        &mut self,
        iova: usize,
        frames: &Vmo,
        offset: usize,
        length: usize,
        permissions: u64,
    ) -> Result<(), Error>;
    fn remove(&mut self, iova: usize, length: usize) -> Result<(), Error>;
    fn start(&mut self) -> Result<(), Error>;
    fn quiesce(&mut self) -> Result<(), Error>;
}
pub struct UnavailableBackend;
// SAFETY: no operation claims to establish hardware isolation or quiescence.
unsafe impl Backend for UnavailableBackend {
    fn isolate(&mut self) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    fn install(&mut self, _: usize, _: &Vmo, _: usize, _: usize, _: u64) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    fn remove(&mut self, _: usize, _: usize) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    fn start(&mut self) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    fn quiesce(&mut self) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub id: u64,
    pub iova: usize,
    pub length: usize,
}
struct Lease {
    mapping: Mapping,
    _object: Arc<Vmo>,
    _writable: Option<WritableMapping>,
}
pub struct Device<B: Backend> {
    backend: Option<B>,
    lifecycle: Lifecycle,
    leases: [Option<Lease>; abi::MAX_DMA_MAPPINGS],
    bytes: usize,
    next_id: u64,
    next_iova: usize,
}
impl<B: Backend> Device<B> {
    pub fn new(mut backend: B) -> Result<Self, Error> {
        if let Err(error) = backend.isolate() {
            if error != Error::Unsupported {
                QUARANTINED_ASSIGNMENTS.fetch_add(1, Ordering::Relaxed);
                core::mem::forget(backend);
            }
            return Err(error);
        }
        Ok(Self {
            backend: Some(backend),
            lifecycle: Lifecycle::isolated(),
            leases: core::array::from_fn(|_| None),
            bytes: 0,
            next_id: 1,
            next_iova: 4096,
        })
    }
    pub fn state(&self) -> State {
        self.lifecycle.state()
    }
    pub fn dma_bytes(&self) -> usize {
        self.bytes
    }
    pub fn map_dma(
        &mut self,
        object: Arc<Vmo>,
        rights: Rights,
        request: abi::DmaRequest,
    ) -> Result<Mapping, Error> {
        self.lifecycle.configurable()?;
        request.validate().map_err(|_| Error::Invalid)?;
        let mut required = Rights::MAP;
        if request.permissions & abi::DEVICE_READ != 0 {
            required |= Rights::READ;
        }
        if request.permissions & abi::DEVICE_WRITE != 0 {
            required |= Rights::WRITE;
        }
        if !rights.contains(required) {
            return Err(Error::Permission);
        }
        object
            .validate_owned_range(request.offset, request.length)
            .map_err(|_| Error::Invalid)?;
        let bytes = self
            .bytes
            .checked_add(request.length)
            .filter(|n| *n <= abi::MAX_DMA_BYTES)
            .ok_or(Error::Quota)?;
        let slot = self
            .leases
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Quota)?;
        let next_id = self.next_id.checked_add(1).ok_or(Error::Quota)?;
        let next_iova = self
            .next_iova
            .checked_add(request.length)
            .ok_or(Error::Quota)?;
        let writable = object
            .mapping_permit(if request.permissions & abi::DEVICE_WRITE != 0 {
                MapFlags::user_rw()
            } else {
                MapFlags::user_ro()
            })
            .map_err(|_| Error::Permission)?;
        let mapping = Mapping {
            id: self.next_id,
            iova: self.next_iova,
            length: request.length,
        };
        self.leases[slot] = Some(Lease {
            mapping,
            _object: Arc::clone(&object),
            _writable: writable,
        });
        self.bytes = bytes;
        self.next_id = next_id;
        self.next_iova = next_iova;
        if let Err(error) = self.backend.as_mut().expect("owned backend").install(
            mapping.iova,
            &object,
            request.offset,
            request.length,
            request.permissions,
        ) {
            self.lifecycle.quarantine();
            return Err(error);
        }
        Ok(mapping)
    }
    pub fn unmap_dma(&mut self, id: u64) -> Result<(), Error> {
        self.lifecycle.configurable()?;
        let slot = self
            .leases
            .iter()
            .position(|l| l.as_ref().is_some_and(|l| l.mapping.id == id))
            .ok_or(Error::Invalid)?;
        let mapping = self.leases[slot].as_ref().unwrap().mapping;
        if let Err(error) = self
            .backend
            .as_mut()
            .expect("owned backend")
            .remove(mapping.iova, mapping.length)
        {
            self.lifecycle.quarantine();
            return Err(error);
        }
        self.bytes -= mapping.length;
        drop(self.leases[slot].take());
        Ok(())
    }
    pub fn start(&mut self) -> Result<(), Error> {
        self.lifecycle.configurable()?;
        if self.bytes == 0 {
            return Err(Error::Invalid);
        }
        if let Err(error) = self.backend.as_mut().expect("owned backend").start() {
            self.lifecycle.quarantine();
            return Err(error);
        }
        self.lifecycle.started()
    }
    pub fn stop(&mut self) -> Result<(), Error> {
        if matches!(self.state(), State::Quarantined | State::Revoked) {
            return self.lifecycle.configurable();
        }
        if let Err(error) = self.backend.as_mut().expect("owned backend").quiesce() {
            self.lifecycle.quarantine();
            return Err(error);
        }
        self.lifecycle.stopped()
    }
    pub fn revoke(&mut self) -> Result<(), Error> {
        self.stop()?;
        self.lifecycle.revoke()
    }
}
impl<B: Backend> Drop for Device<B> {
    fn drop(&mut self) {
        let safe = match self.state() {
            State::Revoked => true,
            State::Quarantined => false,
            _ => self
                .backend
                .as_mut()
                .expect("owned backend")
                .quiesce()
                .is_ok(),
        };
        if !safe {
            QUARANTINED_ASSIGNMENTS.fetch_add(1, Ordering::Relaxed);
            QUARANTINED_DMA_BYTES.fetch_add(self.bytes, Ordering::Relaxed);
            // Translation tables and IRQ routing storage must survive uncertain hardware too.
            if let Some(backend) = self.backend.take() {
                core::mem::forget(backend);
            }
            // A failed hardware drain never permits reuse of pinned frames or write permits.
            for lease in &mut self.leases {
                if let Some(lease) = lease.take() {
                    core::mem::forget(lease);
                }
            }
        }
    }
}
