//! Approved device pages remain distinct from allocator-owned RAM VMOs.
#![allow(dead_code)]
use super::{abi, Error};

#[derive(Clone, Copy, Debug)]
pub struct Region {
    physical: usize,
    info: abi::RegionInfo,
}
impl Region {
    /// # Safety
    /// The entire page extent must belong to this assignment, contain no RAM,
    /// privileged control registers or MSI-X table, and have no conflicting cache alias.
    /// Platform discovery must retain exclusive ownership until all mappings retire.
    pub unsafe fn new(physical: usize, info: abi::RegionInfo) -> Result<Self, Error> {
        info.validate().map_err(|_| Error::Invalid)?;
        if physical & 4095 != 0 || physical.checked_add(info.length).is_none() {
            return Err(Error::Invalid);
        }
        Ok(Self { physical, info })
    }
    pub fn info(&self) -> abi::RegionInfo {
        self.info
    }
    /// Only the complete approved region can be mapped; callers never select subpages.
    pub fn authorize(&self, request: abi::MapRequest) -> Result<usize, Error> {
        request.validate().map_err(|_| Error::Invalid)?;
        if request.length != self.info.length {
            return Err(Error::Invalid);
        }
        if request.permissions & !self.info.permissions != 0 {
            return Err(Error::Permission);
        }
        Ok(self.physical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_pages_do_not_allow_partial_or_stronger_mappings() {
        let info = abi::RegionInfo {
            kind: abi::REGION_REGISTERS,
            length: 8192,
            permissions: abi::MAP_READ,
            width: 0,
            height: 0,
            stride: 0,
            format: 0,
        };
        // SAFETY: synthetic addresses are never dereferenced in this host test.
        let region = unsafe { Region::new(0x1000_0000, info).unwrap() };
        let request = abi::MapRequest {
            region: 0,
            address: 0x4000,
            length: 8192,
            permissions: abi::MAP_READ,
        };
        assert_eq!(region.authorize(request), Ok(0x1000_0000));
        assert_eq!(
            region.authorize(abi::MapRequest {
                length: 4096,
                ..request
            }),
            Err(Error::Invalid)
        );
        assert_eq!(
            region.authorize(abi::MapRequest {
                permissions: 3,
                ..request
            }),
            Err(Error::Permission)
        );
        assert!(unsafe { Region::new(1, info) }.is_err());
        assert!(unsafe { Region::new(usize::MAX & !4095, info) }.is_err());
    }
}
