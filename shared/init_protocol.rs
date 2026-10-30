//! Kernel-to-init grants, independent of the stack startup record.
#![allow(dead_code)]
pub const SIZE: usize = 32;
pub const START: u16 = 1;
pub const READY: u16 = 2;
const MAGIC: &[u8; 8] = b"FRINIT03";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolError;
pub fn encode(opcode: u16) -> Result<[u8; SIZE], ProtocolError> {
    encode_length(opcode, 0)
}
pub fn encode_length(opcode: u16, service_len: u64) -> Result<[u8; SIZE], ProtocolError> {
    if !matches!(opcode, START | READY) || opcode == READY && service_len > 9 {
        return Err(ProtocolError);
    }
    let mut b = [0; SIZE];
    b[..8].copy_from_slice(MAGIC);
    b[8..10].copy_from_slice(&3u16.to_le_bytes());
    b[10..12].copy_from_slice(&opcode.to_le_bytes());
    b[16..24].copy_from_slice(&service_len.to_le_bytes());
    Ok(b)
}
pub fn decode_record(b: &[u8]) -> Result<(u16, u64), ProtocolError> {
    if b.len() != SIZE
        || &b[..8] != MAGIC
        || b[8..10] != 3u16.to_le_bytes()
        || b[12..16] != [0; 4]
        || b[24..32] != [0; 8]
    {
        return Err(ProtocolError);
    }
    let op = u16::from_le_bytes([b[10], b[11]]);
    let len = u64::from_le_bytes(b[16..24].try_into().unwrap());
    if !matches!(op, START | READY) || op == READY && len > 9 {
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
    fn exact_wire_format_and_ready_counts() {
        for op in [START, READY] {
            let b = encode(op).unwrap();
            assert_eq!(&b[..10], b"FRINIT03\x03\0");
            assert_eq!(decode(&b), Ok(op));
        }
        for count in 0..=9 {
            assert_eq!(
                decode_record(&encode_length(READY, count).unwrap()),
                Ok((READY, count))
            );
        }
        for count in [10, u64::MAX] {
            assert!(encode_length(READY, count).is_err());
        }
        assert!(encode(3).is_err());
        assert_eq!(
            decode_record(&encode_length(START, 123).unwrap()),
            Ok((START, 123))
        );
    }
    #[test]
    fn rejects_invalid_metadata_but_preserves_payload_fields() {
        let b = encode_length(READY, 3).unwrap();
        for n in 0..SIZE {
            assert!(decode(&b[..n]).is_err());
            let mut bad = b;
            bad[n] ^= 0x80;
            assert!(decode(&bad).is_err(), "byte {n}");
        }
        // The payload is a count for READY and an arbitrary source length for START.
        let mut ready = b;
        ready[16] = 9;
        assert_eq!(decode_record(&ready), Ok((READY, 9)));
        ready[16] = 10;
        assert!(decode_record(&ready).is_err());
        let mut start = encode(START).unwrap();
        start[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(decode_record(&start), Ok((START, u64::MAX)));
        assert!(decode(&[0; SIZE + 1]).is_err());
    }
}
