//! Concrete device capabilities retain one driver owner independently of handle slots.
#![allow(dead_code)]
use super::{abi, region::Region, Backend, Device, Error, Mapping, State};
use crate::kernel::{
    capabilities::{KernelObject, Rights},
    memory::vmo::Vmo,
    sync::SpinLock,
};
use alloc::{boxed::Box, sync::Arc};

pub const MAX_REGIONS: usize = 8;
struct Inner {
    device: Device<Box<dyn Backend>>,
    owner: Option<u64>,
}
pub struct Assignment {
    inner: SpinLock<Inner>,
    regions: [Option<Region>; MAX_REGIONS],
}
impl KernelObject for Assignment {
    fn type_name(&self) -> &'static str {
        "Device"
    }
}
impl Assignment {
    pub fn new<B: Backend + 'static>(backend: B, regions: &[Region]) -> Result<Self, Error> {
        if regions.len() > MAX_REGIONS {
            return Err(Error::Quota);
        }
        let backend: Box<dyn Backend> = Box::try_new(backend).map_err(|_| Error::Quota)?;
        let device = Device::new(backend)?;
        Ok(Self {
            inner: SpinLock::new(Inner {
                device,
                owner: None,
            }),
            regions: core::array::from_fn(|index| regions.get(index).copied()),
        })
    }
    pub fn region(&self, index: usize) -> Result<Region, Error> {
        self.regions
            .get(index)
            .copied()
            .flatten()
            .ok_or(Error::Invalid)
    }
    pub fn state(&self) -> State {
        self.inner.lock().device.state()
    }
    pub fn dma_bytes(&self) -> usize {
        self.inner.lock().device.dma_bytes()
    }
    /// Validate a prospective owner without capturing the assignment.
    pub fn configurable(&self, process_id: u64) -> Result<(), Error> {
        let inner = self.inner.lock();
        inner.check_claim(process_id)?;
        inner.device.configurable()
    }
    pub fn owner(&self) -> Option<u64> {
        self.inner.lock().owner
    }
    /// Reserve process-owned cleanup storage before claiming authority.
    pub fn claim(&self, process_id: u64) -> Result<(), Error> {
        let mut inner = self.inner.lock();
        inner.device.configurable()?;
        inner.claim(process_id)
    }
    /// The caller reserves its mapping and cleanup records before this operation.
    pub fn authorize_mapping(
        &self,
        process_id: u64,
        index: usize,
        request: abi::MapRequest,
    ) -> Result<usize, Error> {
        if index != request.region as usize {
            return Err(Error::Invalid);
        }
        let region = self.region(index)?;
        let physical = region.authorize(request)?;
        let mut inner = self.inner.lock();
        inner.device.configurable()?;
        inner.claim(process_id)?;
        Ok(physical)
    }
    pub fn map_dma(
        &self,
        process_id: u64,
        object: Arc<Vmo>,
        rights: Rights,
        request: abi::DmaRequest,
    ) -> Result<Mapping, Error> {
        let mut inner = self.inner.lock();
        inner.require_owner(process_id)?;
        inner.device.map_dma(object, rights, request)
    }
    pub fn unmap_dma(&self, process_id: u64, id: u64) -> Result<(), Error> {
        let mut inner = self.inner.lock();
        inner.require_owner(process_id)?;
        inner.device.unmap_dma(id)
    }
    pub fn start(&self, process_id: u64) -> Result<(), Error> {
        let mut inner = self.inner.lock();
        inner.require_owner(process_id)?;
        inner.device.start()
    }
    pub fn stop(&self, process_id: u64) -> Result<(), Error> {
        let mut inner = self.inner.lock();
        inner.require_owner(process_id)?;
        inner.device.stop()
    }
    /// Kernel containment may revoke an assignment even before a driver claims it.
    pub(crate) fn revoke_kernel(&self) -> Result<(), Error> {
        self.inner.lock().device.revoke()
    }
    pub fn revoke(&self, process_id: u64) -> Result<(), Error> {
        let mut inner = self.inner.lock();
        inner.require_owner(process_id)?;
        inner.device.revoke()
    }
}
impl Inner {
    fn check_claim(&self, process_id: u64) -> Result<(), Error> {
        if process_id == 0 {
            return Err(Error::Invalid);
        }
        match self.owner {
            None => Ok(()),
            Some(owner) if owner == process_id => Ok(()),
            _ => Err(Error::Permission),
        }
    }
    fn claim(&mut self, process_id: u64) -> Result<(), Error> {
        self.check_claim(process_id)?;
        self.owner = Some(process_id);
        Ok(())
    }
    fn require_owner(&self, process_id: u64) -> Result<(), Error> {
        if self.owner == Some(process_id) {
            Ok(())
        } else {
            Err(Error::Permission)
        }
    }
}
