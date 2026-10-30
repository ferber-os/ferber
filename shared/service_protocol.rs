//! Resident service messages, with one integer challenge.
#![allow(dead_code)]
pub const SIZE: usize = 16;
pub const READY: u16 = 1;
pub const PING: u16 = 2;
pub const PONG: u16 = 3;
pub const SHUTDOWN: u16 = 4;
pub fn encode(op: u16, value: u32) -> [u8; SIZE] {
    let mut b = [0; SIZE];
    b[..8].copy_from_slice(b"FRSVC001");
    b[8..10].copy_from_slice(&1u16.to_le_bytes());
    b[10..12].copy_from_slice(&op.to_le_bytes());
    b[12..].copy_from_slice(&value.to_le_bytes());
    b
}
pub fn decode(b: &[u8]) -> Option<(u16, u32)> {
    if b.len() != SIZE || &b[..8] != b"FRSVC001" || b[8..10] != 1u16.to_le_bytes() {
        return None;
    }
    let op = u16::from_le_bytes([b[10], b[11]]);
    if !matches!(op, READY | PING | PONG | SHUTDOWN) {
        return None;
    }
    Some((op, u32::from_le_bytes(b[12..].try_into().unwrap())))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_records() {
        for op in [READY, PING, PONG, SHUTDOWN] {
            let b = encode(op, 74);
            assert_eq!(decode(&b), Some((op, 74)));
            for n in 0..SIZE {
                assert!(decode(&b[..n]).is_none());
            }
            for n in 0..12 {
                let mut bad = b;
                bad[n] ^= 0x80;
                assert!(decode(&bad).is_none());
            }
        }
    }
}
