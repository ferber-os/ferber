//! Device DMA requests use exact bytes, never native Rust struct layout.
#![allow(dead_code)]
pub const VERSION: u32 = 1;
pub const DMA_REQUEST_SIZE: usize = 32;
pub const MAX_DMA_MAPPINGS: usize = 64;
pub const MAX_DMA_BYTES: usize = 4 * 1024 * 1024;
pub const DEVICE_READ: u64 = 1;
pub const DEVICE_WRITE: u64 = 2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRequest {
    pub offset: usize,
    pub length: usize,
    pub permissions: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbiError;
impl DmaRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self, AbiError> {
        if bytes.len() != DMA_REQUEST_SIZE {
            return Err(AbiError);
        }
        let at = |n| u64::from_le_bytes(bytes[n..n + 8].try_into().unwrap());
        let request = Self {
            offset: usize::try_from(at(0)).map_err(|_| AbiError)?,
            length: usize::try_from(at(8)).map_err(|_| AbiError)?,
            permissions: at(24),
        };
        if at(16) != 0 {
            return Err(AbiError);
        }
        request.validate()?;
        Ok(request)
    }
    pub fn validate(self) -> Result<(), AbiError> {
        if self.length == 0
            || self.length > MAX_DMA_BYTES
            || (self.offset | self.length) & 4095 != 0
            || self.permissions == 0
            || self.permissions & !(DEVICE_READ | DEVICE_WRITE) != 0
            || self.offset.checked_add(self.length).is_none()
        {
            return Err(AbiError);
        }
        Ok(())
    }
    pub fn encode(self) -> Result<[u8; DMA_REQUEST_SIZE], AbiError> {
        self.validate()?;
        let mut b = [0; DMA_REQUEST_SIZE];
        for (i, n) in [self.offset as u64, self.length as u64, 0, self.permissions]
            .into_iter()
            .enumerate()
        {
            b[i * 8..i * 8 + 8].copy_from_slice(&n.to_le_bytes());
        }
        Ok(b)
    }
}

// Device regions are selected by an opaque index, never a physical address.
pub const MAP_REQUEST_SIZE: usize = 32;
pub const MAP_READ: u64 = 1;
pub const MAP_WRITE: u64 = 2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapRequest {
    pub region: u32,
    pub address: usize,
    pub length: usize,
    pub permissions: u64,
}
impl MapRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self, AbiError> {
        if bytes.len() != MAP_REQUEST_SIZE || bytes[4..8] != [0; 4] {
            return Err(AbiError);
        }
        let at = |n| u64::from_le_bytes(bytes[n..n + 8].try_into().unwrap());
        let request = Self {
            region: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            address: usize::try_from(at(8)).map_err(|_| AbiError)?,
            length: usize::try_from(at(16)).map_err(|_| AbiError)?,
            permissions: at(24),
        };
        request.validate()?;
        Ok(request)
    }
    pub fn validate(self) -> Result<(), AbiError> {
        if self.address == 0
            || self.length == 0
            || (self.address | self.length) & 4095 != 0
            || !matches!(self.permissions, MAP_READ | 3)
            || self.address.checked_add(self.length).is_none()
        {
            return Err(AbiError);
        }
        Ok(())
    }
    pub fn encode(self) -> Result<[u8; MAP_REQUEST_SIZE], AbiError> {
        self.validate()?;
        let mut bytes = [0; MAP_REQUEST_SIZE];
        bytes[..4].copy_from_slice(&self.region.to_le_bytes());
        for (i, value) in [self.address as u64, self.length as u64, self.permissions]
            .into_iter()
            .enumerate()
        {
            bytes[8 + i * 8..16 + i * 8].copy_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    #[test]
    fn mapping_wire_is_exact_and_never_requests_execution() {
        let request = MapRequest {
            region: 2,
            address: 0x4000,
            length: 8192,
            permissions: 3,
        };
        let bytes = request.encode().unwrap();
        assert_eq!(MapRequest::decode(&bytes), Ok(request));
        for length in 0..MAP_REQUEST_SIZE {
            assert!(MapRequest::decode(&bytes[..length]).is_err());
        }
        let mut reserved = bytes;
        reserved[4] = 1;
        assert!(MapRequest::decode(&reserved).is_err());
        for permissions in [0, MAP_WRITE, 4, 5, 7, u64::MAX] {
            assert!(MapRequest {
                permissions,
                ..request
            }
            .validate()
            .is_err());
        }
        for address in [0, 1, usize::MAX & !4095] {
            assert!(MapRequest { address, ..request }.validate().is_err());
        }
        for length in [0, 1, 4097] {
            assert!(MapRequest { length, ..request }.validate().is_err());
        }
    }
}

pub const SYS_DEVICE_QUERY: usize = 24;
pub const SYS_DEVICE_MAP: usize = 25;
pub const SYS_DEVICE_UNMAP: usize = 26;
pub const SYS_DMA_MAP: usize = 27;
pub const SYS_DMA_UNMAP: usize = 28;
pub const SYS_IRQ_BIND: usize = 29;
pub const SYS_IRQ_ACK: usize = 30;
pub const SYS_DEVICE_CONTROL: usize = 31;
pub const CONTROL_START: usize = 1;
pub const CONTROL_STOP: usize = 2;
pub const CONTROL_TIME: usize = 3;
pub const CONTROL_DEADLINE: usize = 4;
pub const REGION_INFO_SIZE: usize = 48;
pub const REGION_REGISTERS: u32 = 1;
pub const REGION_FRAMEBUFFER: u32 = 2;
pub const REGION_UART: u32 = 3;
pub const PIXEL_RGB: u32 = 1;
pub const PIXEL_BGR: u32 = 2;

/// Query returns geometry and access rights, without disclosing a physical address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionInfo {
    pub kind: u32,
    pub length: usize,
    pub permissions: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}
impl RegionInfo {
    pub fn validate(self) -> Result<(), AbiError> {
        if self.length == 0 || self.length & 4095 != 0 || !matches!(self.permissions, MAP_READ | 3)
        {
            return Err(AbiError);
        }
        match self.kind {
            REGION_REGISTERS | REGION_UART => {
                if self.width | self.height | self.stride | self.format != 0 {
                    return Err(AbiError);
                }
            }
            REGION_FRAMEBUFFER => {
                if self.width == 0
                    || self.height == 0
                    || self.stride < self.width
                    || !matches!(self.format, PIXEL_RGB | PIXEL_BGR)
                    || (self.stride as u64)
                        .checked_mul(self.height as u64)
                        .and_then(|n| n.checked_mul(4))
                        .is_none_or(|n| n > self.length as u64)
                {
                    return Err(AbiError);
                }
            }
            _ => return Err(AbiError),
        }
        Ok(())
    }
    pub fn encode(self) -> Result<[u8; REGION_INFO_SIZE], AbiError> {
        self.validate()?;
        let mut bytes = [0; REGION_INFO_SIZE];
        bytes[..4].copy_from_slice(&VERSION.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.kind.to_le_bytes());
        bytes[8..16].copy_from_slice(&(self.length as u64).to_le_bytes());
        bytes[16..24].copy_from_slice(&self.permissions.to_le_bytes());
        for (i, value) in [self.width, self.height, self.stride, self.format]
            .into_iter()
            .enumerate()
        {
            bytes[24 + i * 4..28 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, AbiError> {
        if bytes.len() != REGION_INFO_SIZE
            || bytes[..4] != VERSION.to_le_bytes()
            || bytes[40..48] != [0; 8]
        {
            return Err(AbiError);
        }
        let word = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
        let info = Self {
            kind: word(4),
            length: usize::try_from(u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
                .map_err(|_| AbiError)?,
            permissions: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            width: word(24),
            height: word(28),
            stride: word(32),
            format: word(36),
        };
        info.validate()?;
        Ok(info)
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;
    #[test]
    fn framebuffer_geometry_and_reserved_bytes_are_checked() {
        let info = RegionInfo {
            kind: REGION_FRAMEBUFFER,
            length: 8 * 1024 * 1024,
            permissions: 3,
            width: 1920,
            height: 1080,
            stride: 1920,
            format: PIXEL_BGR,
        };
        let bytes = info.encode().unwrap();
        assert_eq!(RegionInfo::decode(&bytes), Ok(info));
        let mut bad = bytes;
        bad[40] = 1;
        assert!(RegionInfo::decode(&bad).is_err());
        for invalid in [
            RegionInfo {
                length: 4096,
                ..info
            },
            RegionInfo {
                stride: 1919,
                ..info
            },
            RegionInfo { format: 0, ..info },
            RegionInfo {
                kind: REGION_UART,
                ..info
            },
            RegionInfo {
                permissions: 7,
                ..info
            },
        ] {
            assert!(invalid.encode().is_err());
        }
    }
}
