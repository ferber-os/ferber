//! Versioned startup bytes shared by the kernel and independently linked programs.
#![allow(dead_code)]

pub const MAGIC: u64 = 0x3154_5241_5453_5246; // "FRSTART1" in little-endian order.
pub const VERSION: u32 = 1;
pub const SIZE: usize = 32;
pub const BOOTSTRAP_PRESENT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Startup {
    pub bootstrap: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupError;

impl Startup {
    pub fn encode(self) -> [u8; SIZE] {
        let mut bytes = [0; SIZE];
        bytes[0..8].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[8..12].copy_from_slice(&VERSION.to_le_bytes());
        bytes[12..16].copy_from_slice(&(SIZE as u32).to_le_bytes());
        if let Some(handle) = self.bootstrap {
            bytes[16..20].copy_from_slice(&handle.to_le_bytes());
            bytes[20..24].copy_from_slice(&BOOTSTRAP_PRESENT.to_le_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StartupError> {
        if bytes.len() != SIZE {
            return Err(StartupError);
        }
        let u32_at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if u64::from_le_bytes(bytes[0..8].try_into().unwrap()) != MAGIC
            || u32_at(8) != VERSION
            || u32_at(12) != SIZE as u32
            || bytes[24..32] != [0; 8]
        {
            return Err(StartupError);
        }
        let bootstrap = match u32_at(20) {
            0 if u32_at(16) == 0 => None,
            BOOTSTRAP_PRESENT => Some(u32_at(16)),
            _ => return Err(StartupError),
        };
        Ok(Self { bootstrap })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_roundtrips_absent_zero_and_nonzero_handles() {
        for bootstrap in [None, Some(0), Some(7), Some(u32::MAX)] {
            let record = Startup { bootstrap };
            let bytes = record.encode();
            assert_eq!(&bytes[..8], b"FRSTART1");
            assert_eq!(Startup::decode(&bytes), Ok(record));
        }
    }

    #[test]
    fn startup_rejects_wrong_lengths_headers_flags_and_reserved_bytes() {
        let valid = Startup { bootstrap: Some(7) }.encode();
        for len in 0..SIZE {
            assert_eq!(Startup::decode(&valid[..len]), Err(StartupError));
        }
        assert_eq!(Startup::decode(&[0; SIZE + 1]), Err(StartupError));
        for index in (0..16).chain(20..32) {
            let mut bytes = valid;
            bytes[index] ^= 0x80;
            assert_eq!(Startup::decode(&bytes), Err(StartupError), "byte {index}");
        }
        let mut absent = Startup { bootstrap: None }.encode();
        absent[16] = 1;
        assert_eq!(Startup::decode(&absent), Err(StartupError));
    }
}
