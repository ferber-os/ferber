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
