//! Kernel-to-init bootstrap protocol; independent of the stack startup record.
#![allow(dead_code)]

pub const SIZE: usize = 16;
pub const START: u16 = 1;
pub const READY: u16 = 2;
const MAGIC: &[u8; 8] = b"FRINIT01";
const VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolError;

pub fn encode(opcode: u16) -> Result<[u8; SIZE], ProtocolError> {
    if !matches!(opcode, START | READY) {
        return Err(ProtocolError);
    }
    let mut bytes = [0; SIZE];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&opcode.to_le_bytes());
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<u16, ProtocolError> {
    if bytes.len() != SIZE
        || &bytes[..8] != MAGIC
        || bytes[8..10] != VERSION.to_le_bytes()
        || bytes[12..16] != [0; 4]
    {
        return Err(ProtocolError);
    }
    let opcode = u16::from_le_bytes([bytes[10], bytes[11]]);
    if !matches!(opcode, START | READY) {
        return Err(ProtocolError);
    }
    Ok(opcode)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_wire_format() {
        for opcode in [START, READY] {
            let bytes = encode(opcode).unwrap();
            assert_eq!(&bytes[..10], b"FRINIT01\x01\0");
            assert_eq!(decode(&bytes), Ok(opcode));
        }
        assert_eq!(encode(0), Err(ProtocolError));
        assert_eq!(encode(3), Err(ProtocolError));
    }
    #[test]
    fn rejects_every_invalid_field_and_length() {
        let valid = encode(START).unwrap();
        for len in 0..SIZE {
            assert_eq!(decode(&valid[..len]), Err(ProtocolError));
        }
        assert_eq!(decode(&[0; SIZE + 1]), Err(ProtocolError));
        for index in 0..SIZE {
            let mut bytes = valid;
            bytes[index] ^= 0x80;
            assert_eq!(decode(&bytes), Err(ProtocolError), "byte {index}");
        }
    }
}
