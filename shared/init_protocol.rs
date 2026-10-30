//! Kernel-to-init grants, independent of the stack startup record.
#![allow(dead_code)]
pub const SIZE: usize = 32;
pub const START: u16 = 1;
pub const READY: u16 = 2;
const MAGIC: &[u8; 8] = b"FRINIT02";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolError;
pub fn encode(opcode: u16) -> Result<[u8; SIZE], ProtocolError> {
    encode_length(opcode, 0)
}
pub fn encode_length(opcode: u16, service_len: u64) -> Result<[u8; SIZE], ProtocolError> {
    if !matches!(opcode, START | READY) || opcode == READY && service_len != 0 {
        return Err(ProtocolError);
    }
    let mut b = [0; SIZE];
    b[..8].copy_from_slice(MAGIC);
    b[8..10].copy_from_slice(&2u16.to_le_bytes());
    b[10..12].copy_from_slice(&opcode.to_le_bytes());
    b[16..24].copy_from_slice(&service_len.to_le_bytes());
    Ok(b)
}
pub fn decode_record(b: &[u8]) -> Result<(u16, u64), ProtocolError> {
    if b.len() != SIZE
        || &b[..8] != MAGIC
        || b[8..10] != 2u16.to_le_bytes()
        || b[12..16] != [0; 4]
        || b[24..32] != [0; 8]
    {
        return Err(ProtocolError);
    }
    let op = u16::from_le_bytes([b[10], b[11]]);
    let len = u64::from_le_bytes(b[16..24].try_into().unwrap());
    if !matches!(op, START | READY) || op == READY && len != 0 {
        return Err(ProtocolError);
    }
    Ok((op, len))
}
pub fn decode(b: &[u8]) -> Result<u16, ProtocolError> {
    decode_record(b).map(|p| p.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_wire_format() {
        for op in [START, READY] {
            let b = encode(op).unwrap();
            assert_eq!(&b[..10], b"FRINIT02\x02\0");
            assert_eq!(decode(&b), Ok(op));
        }
        assert!(encode(3).is_err());
        assert!(encode_length(READY, 1).is_err());
        assert_eq!(
            decode_record(&encode_length(START, 123).unwrap()),
            Ok((START, 123))
        );
    }
    #[test]
    fn rejects_every_invalid_field_and_length() {
        let b = encode(READY).unwrap();
        for n in 0..SIZE {
            assert!(decode(&b[..n]).is_err());
            let mut bad = b;
            bad[n] ^= 0x80;
            assert!(decode(&bad).is_err(), "byte {n}");
        }
        assert!(decode(&[0; SIZE + 1]).is_err());
    }
}
