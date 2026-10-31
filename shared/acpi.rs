//! Immutable firmware-table snapshot and bounded ACPI readers.
#![allow(dead_code)]

pub const SNAPSHOT_VA: u64 = 0xffff_fffe_0200_0000;
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_TABLES: usize = 8;
pub const MAX_ROOT_ENTRIES: usize = 256;
pub const HEADER: usize = 16;
pub const RECORD: usize = 32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AcpiSnapshot {
    pub physical_start: u64,
    pub virtual_start: u64,
    pub length: u64,
    pub reserved: u64,
}
impl AcpiSnapshot {
    pub const NONE: Self = Self {
        physical_start: 0,
        virtual_start: 0,
        length: 0,
        reserved: 0,
    };
    pub fn bounds(&self) -> Result<Option<(u64, u64)>, Error> {
        if *self == Self::NONE {
            return Ok(None);
        }
        if self.reserved != 0
            || self.physical_start == 0
            || self.physical_start & 4095 != 0
            || self.virtual_start != SNAPSHOT_VA
            || self.length == 0
            || self.length > MAX_BYTES as u64
        {
            return Err(Error);
        }
        let end = self
            .physical_start
            .checked_add(self.length)
            .and_then(|n| n.checked_add(4095))
            .ok_or(Error)?
            & !4095;
        if end > 1u64 << 52 {
            return Err(Error);
        }
        Ok(Some((self.physical_start, end)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error;
pub fn u16_at(b: &[u8], p: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        b.get(p..p.checked_add(2).ok_or(Error)?)
            .ok_or(Error)?
            .try_into()
            .map_err(|_| Error)?,
    ))
}
pub fn u32_at(b: &[u8], p: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        b.get(p..p.checked_add(4).ok_or(Error)?)
            .ok_or(Error)?
            .try_into()
            .map_err(|_| Error)?,
    ))
}
pub fn u64_at(b: &[u8], p: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(
        b.get(p..p.checked_add(8).ok_or(Error)?)
            .ok_or(Error)?
            .try_into()
            .map_err(|_| Error)?,
    ))
}
pub fn checksum(b: &[u8]) -> bool {
    b.iter().fold(0u8, |a, b| a.wrapping_add(*b)) == 0
}
pub fn rsdp(b: &[u8]) -> Result<u64, Error> {
    if b.len() < 36 || &b[..8] != b"RSD PTR " || b[15] < 2 || !checksum(&b[..20]) {
        return Err(Error);
    }
    let size = u32_at(b, 20)? as usize;
    if size != b.len() || size > 4096 || !checksum(b) || b[33..36] != [0; 3] {
        return Err(Error);
    }
    let xsdt = u64_at(b, 24)?;
    if xsdt == 0 {
        return Err(Error);
    }
    Ok(xsdt)
}
pub fn sdt<'a>(b: &'a [u8], signature: &[u8; 4]) -> Result<&'a [u8], Error> {
    if b.len() < 36
        || b.len() > MAX_BYTES
        || &b[..4] != signature
        || u32_at(b, 4)? as usize != b.len()
        || !checksum(b)
    {
        return Err(Error);
    }
    Ok(b)
}
pub fn xsdt(b: &[u8]) -> Result<impl Iterator<Item = u64> + Clone + '_, Error> {
    sdt(b, b"XSDT")?;
    if !(b.len() - 36).is_multiple_of(8) || (b.len() - 36) / 8 > MAX_ROOT_ENTRIES {
        return Err(Error);
    }
    for (i, p) in b[36..].as_chunks::<8>().0.iter().enumerate() {
        let address = u64::from_le_bytes(*p);
        if address == 0 || b[36..36 + i * 8].as_chunks::<8>().0.iter().any(|q| q == p) {
            return Err(Error);
        }
    }
    Ok(b[36..]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|p| u64::from_le_bytes(*p)))
}
#[derive(Clone, Copy)]
pub struct Table<'a> {
    pub signature: [u8; 8],
    pub physical: u64,
    pub bytes: &'a [u8],
}
pub struct Snapshot<'a> {
    bytes: &'a [u8],
    count: usize,
}
impl<'a> Snapshot<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER
            || bytes.len() > MAX_BYTES
            || &bytes[..8] != b"FRACPI01"
            || u32_at(bytes, 8)? as usize != bytes.len()
        {
            return Err(Error);
        }
        let count = u32_at(bytes, 12)? as usize;
        if !(2..=MAX_TABLES).contains(&count) {
            return Err(Error);
        }
        let metadata = HEADER
            .checked_add(count.checked_mul(RECORD).ok_or(Error)?)
            .ok_or(Error)?;
        if metadata > bytes.len() {
            return Err(Error);
        }
        let mut end = metadata;
        for i in 0..count {
            let r = HEADER + i * RECORD;
            let sig: [u8; 8] = bytes[r..r + 8].try_into().map_err(|_| Error)?;
            let physical = u64_at(bytes, r + 8)?;
            let offset = u32_at(bytes, r + 16)? as usize;
            let len = u32_at(bytes, r + 20)? as usize;
            let aligned = end.checked_add(7).ok_or(Error)? & !7;
            let next = offset.checked_add(len).ok_or(Error)?;
            if physical == 0
                || physical.checked_add(len as u64).is_none()
                || u64_at(bytes, r + 24)? != 0
                || offset != aligned
                || next > bytes.len()
                || bytes[end..offset].iter().any(|b| *b != 0)
                || (0..i).any(|j| bytes[HEADER + j * RECORD..HEADER + j * RECORD + 8] == sig)
            {
                return Err(Error);
            }
            let data = &bytes[offset..next];
            if i == 0 {
                if &sig != b"RSD PTR " {
                    return Err(Error);
                }
                rsdp(data)?;
            } else {
                if sig[4..] != [0; 4]
                    || !matches!(&sig[..4], b"XSDT" | b"APIC" | b"MCFG" | b"HPET" | b"IVRS")
                {
                    return Err(Error);
                }
                sdt(data, sig[..4].try_into().map_err(|_| Error)?)?;
                if i == 1 && &sig[..4] != b"XSDT" {
                    return Err(Error);
                }
            }
            end = next;
        }
        if end != bytes.len() {
            return Err(Error);
        }
        let snapshot = Self { bytes, count };
        let root = snapshot.table(b"RSD PTR ").ok_or(Error)?;
        let x = snapshot.table(b"XSDT\0\0\0\0").ok_or(Error)?;
        if rsdp(root.bytes)? != x.physical {
            return Err(Error);
        }
        let addresses = xsdt(x.bytes)?;
        for t in snapshot.tables().skip(2) {
            if !addresses.clone().any(|address| address == t.physical) {
                return Err(Error);
            }
        }
        Ok(snapshot)
    }
    pub fn tables(&self) -> impl Iterator<Item = Table<'a>> + '_ {
        (0..self.count).map(|i| {
            let r = HEADER + i * RECORD;
            let offset = u32_at(self.bytes, r + 16).expect("validated snapshot offset") as usize;
            let len = u32_at(self.bytes, r + 20).expect("validated snapshot length") as usize;
            Table {
                signature: self.bytes[r..r + 8]
                    .try_into()
                    .expect("validated signature"),
                physical: u64_at(self.bytes, r + 8).expect("validated physical address"),
                bytes: &self.bytes[offset..offset + len],
            }
        })
    }
    pub fn table(&self, signature: &[u8; 8]) -> Option<Table<'a>> {
        self.tables().find(|t| &t.signature == signature)
    }
}
