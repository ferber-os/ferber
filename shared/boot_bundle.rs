//! An initial executable and opaque initramfs payload; independent of the firmware handoff.
#![allow(dead_code)]
pub const MAGIC: &[u8; 8] = b"FRBOOT01";
pub const HEADER: usize = 32;
pub const ENTRY: usize = 32;
pub const MAX_SIZE: usize = 4 * 1024 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleError;
pub struct Bundle<'a> {
    pub init: &'a [u8],
    pub archive: &'a [u8],
}
impl<'a> Bundle<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, BundleError> {
        if bytes.len() < HEADER + 2 * ENTRY || bytes.len() > MAX_SIZE || &bytes[..8] != MAGIC {
            return Err(BundleError);
        }
        let u32_at = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
        let u64_at = |n| u64::from_le_bytes(bytes[n..n + 8].try_into().unwrap());
        if u32_at(8) != 2 || u32_at(12) != 2 || u64_at(16) != bytes.len() as u64 || u64_at(24) != 0
        {
            return Err(BundleError);
        }
        let mut result = [&bytes[0..0]; 2];
        let mut end = HEADER + 2 * ENTRY;
        for (index, slot) in result.iter_mut().enumerate() {
            let base = HEADER + index * ENTRY;
            if u32_at(base) != if index == 0 { 1 } else { 3 }
                || u32_at(base + 4) != 0
                || u64_at(base + 24) != 0
            {
                return Err(BundleError);
            }
            let offset = usize::try_from(u64_at(base + 8)).map_err(|_| BundleError)?;
            let length = usize::try_from(u64_at(base + 16)).map_err(|_| BundleError)?;
            let next = offset.checked_add(length).ok_or(BundleError)?;
            if offset & 7 != 0
                || offset < end
                || length == 0
                || next > bytes.len()
                || bytes[end..offset].iter().any(|b| *b != 0)
            {
                return Err(BundleError);
            }
            *slot = &bytes[offset..next];
            end = next;
        }
        if end != bytes.len() {
            return Err(BundleError);
        }
        Ok(Self {
            init: result[0],
            archive: result[1],
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 105];
        b[..8].copy_from_slice(MAGIC);
        b[8..12].copy_from_slice(&2u32.to_le_bytes());
        b[12..16].copy_from_slice(&2u32.to_le_bytes());
        b[16..24].copy_from_slice(&105u64.to_le_bytes());
        for (i, offset, len) in [(0, 96u64, 1u64), (1, 104, 1)] {
            let n = 32 + i * 32;
            b[n..n + 4].copy_from_slice(&(if i == 0 { 1u32 } else { 3u32 }).to_le_bytes());
            b[n + 8..n + 16].copy_from_slice(&offset.to_le_bytes());
            b[n + 16..n + 24].copy_from_slice(&len.to_le_bytes());
        }
        b[96] = 1;
        b[104] = 2;
        b
    }
    #[test]
    fn exact_bundle_and_mutations() {
        let b = fixture();
        let p = Bundle::parse(&b).unwrap();
        assert_eq!(p.init, [1]);
        assert_eq!(p.archive, [2]);
        for n in (0..96).chain(97..104) {
            let mut bad = b.clone();
            bad[n] ^= 0x80;
            assert!(Bundle::parse(&bad).is_err(), "byte {n}");
        }
        for n in 0..b.len() {
            assert!(Bundle::parse(&b[..n]).is_err());
        }
        let mut extra = b;
        extra.push(0);
        assert!(Bundle::parse(&extra).is_err());
    }
}
