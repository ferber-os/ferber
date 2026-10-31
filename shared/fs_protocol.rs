//! Initramfs filesystem sessions: IDs are meaningful only on their owning channel.
#![allow(dead_code)]
pub const HEADER: usize = 32;
pub const CHUNK: usize = 4096;
pub const MAX_MESSAGE: usize = HEADER + CHUNK;
pub const MAX_SESSIONS: usize = 9;
pub const MAX_OPEN: usize = 8;
pub const OPEN: u16 = 1;
pub const READ: u16 = 2;
pub const LIST: u16 = 3;
pub const CLOSE: u16 = 4;
pub const STAT: u16 = 5;
pub const FILE: u32 = 1;
pub const DIRECTORY: u32 = 2;
pub const EOF: u32 = 1;
pub const FS_START_SIZE: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub op: u16,
    pub request: u32,
    pub status: i32,
    pub flags: u32,
    pub object: u64,
}
pub fn encode(h: Header, out: &mut [u8]) {
    out[..HEADER].fill(0);
    out[..8].copy_from_slice(b"FRFS0001");
    out[8..10].copy_from_slice(&1u16.to_le_bytes());
    out[10..12].copy_from_slice(&h.op.to_le_bytes());
    out[12..16].copy_from_slice(&h.request.to_le_bytes());
    out[16..20].copy_from_slice(&h.status.to_le_bytes());
    out[20..24].copy_from_slice(&h.flags.to_le_bytes());
    out[24..32].copy_from_slice(&h.object.to_le_bytes());
}
pub fn decode(b: &[u8]) -> Option<Header> {
    if b.len() < HEADER
        || b.len() > MAX_MESSAGE
        || &b[..8] != b"FRFS0001"
        || b[8..10] != 1u16.to_le_bytes()
    {
        return None;
    }
    let h = Header {
        op: u16::from_le_bytes(b[10..12].try_into().ok()?),
        request: u32::from_le_bytes(b[12..16].try_into().ok()?),
        status: i32::from_le_bytes(b[16..20].try_into().ok()?),
        flags: u32::from_le_bytes(b[20..24].try_into().ok()?),
        object: u64::from_le_bytes(b[24..32].try_into().ok()?),
    };
    matches!(h.op, OPEN | READ | LIST | CLOSE | STAT).then_some(h)
}
pub fn fs_start(len: u64, slots: &[u32]) -> Option<[u8; FS_START_SIZE]> {
    if slots.is_empty() || slots.len() > MAX_SESSIONS || len == 0 {
        return None;
    }
    let mut b = [0; FS_START_SIZE];
    b[..8].copy_from_slice(b"FRFSBOOT");
    b[8..16].copy_from_slice(&len.to_le_bytes());
    b[16..20].copy_from_slice(&(slots.len() as u32).to_le_bytes());
    for (i, s) in slots.iter().enumerate() {
        b[20 + i * 4..24 + i * 4].copy_from_slice(&s.to_le_bytes());
    }
    Some(b)
}
pub fn decode_fs_start(b: &[u8]) -> Option<(usize, [u32; MAX_SESSIONS], usize)> {
    if b.len() != FS_START_SIZE || &b[..8] != b"FRFSBOOT" {
        return None;
    }
    let len = usize::try_from(u64::from_le_bytes(b[8..16].try_into().ok()?)).ok()?;
    let n = u32::from_le_bytes(b[16..20].try_into().ok()?) as usize;
    if len == 0 || n == 0 || n > MAX_SESSIONS || b[20 + n * 4..].iter().any(|x| *x != 0) {
        return None;
    }
    let mut slots = [0; MAX_SESSIONS];
    for i in 0..n {
        slots[i] = u32::from_le_bytes(b[20 + i * 4..24 + i * 4].try_into().ok()?);
        if slots[..i].contains(&slots[i]) {
            return None;
        }
    }
    Some((len, slots, n))
}
pub fn service_start(slot: Option<u32>) -> [u8; 16] {
    let mut b = [0; 16];
    b[..8].copy_from_slice(b"FRSVBOOT");
    b[8..12].copy_from_slice(&slot.unwrap_or(u32::MAX).to_le_bytes());
    b
}
pub fn decode_service_start(b: &[u8]) -> Option<Option<u32>> {
    if b.len() != 16 || &b[..8] != b"FRSVBOOT" || b[12..] != [0; 4] {
        return None;
    }
    let s = u32::from_le_bytes(b[8..12].try_into().ok()?);
    Some((s != u32::MAX).then_some(s))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records() {
        let h = Header {
            op: OPEN,
            request: 3,
            status: 0,
            flags: FILE,
            object: 0,
        };
        let mut b = [0; HEADER];
        encode(h, &mut b);
        assert_eq!(decode(&b), Some(h));
        for n in 0..HEADER {
            assert!(decode(&b[..n]).is_none());
        }
        let start = fs_start(14, &[1, 2]).unwrap();
        assert_eq!(decode_fs_start(&start).unwrap().2, 2);
        assert!(decode_fs_start(&fs_start(14, &[1, 1]).unwrap()).is_none());
        assert_eq!(decode_service_start(&service_start(None)), Some(None));
    }
}
