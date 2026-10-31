//! ACPI platform descriptions; parsing never touches hardware registers.
#![allow(dead_code)]
use super::acpi::{self, u16_at, u32_at, u64_at, Error};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ecam {
    pub base: u64,
    pub segment: u16,
    pub first_bus: u8,
    pub last_bus: u8,
}
impl Ecam {
    pub fn address(&self, bus: u8, device: u8, function: u8, register: u16) -> Result<u64, Error> {
        if bus < self.first_bus
            || bus > self.last_bus
            || device >= 32
            || function >= 8
            || register >= 4096
            || register & 3 != 0
        {
            return Err(Error);
        }
        self.base
            .checked_add(
                ((bus as u64) << 20)
                    | ((device as u64) << 15)
                    | ((function as u64) << 12)
                    | register as u64,
            )
            .ok_or(Error)
    }
    pub fn end(&self) -> Result<u64, Error> {
        self.base
            .checked_add((self.last_bus as u64 + 1) << 20)
            .ok_or(Error)
    }
}
pub struct Mcfg<'a> {
    bytes: &'a [u8],
}
impl<'a> Mcfg<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        acpi::sdt(bytes, b"MCFG")?;
        if bytes.len() < 60
            || !(bytes.len() - 44).is_multiple_of(16)
            || (bytes.len() - 44) / 16 > 16
            || bytes[36..44] != [0; 8]
        {
            return Err(Error);
        }
        let result = Self { bytes };
        for (i, e) in result.regions().enumerate() {
            if e.base == 0
                || e.base & ((1 << 20) - 1) != 0
                || e.first_bus > e.last_bus
                || e.end()? > (1u64 << 52)
            {
                return Err(Error);
            }
            let r = 44 + i * 16;
            if bytes[r + 12..r + 16] != [0; 4] {
                return Err(Error);
            }
            for previous in result.regions().take(i) {
                if previous.segment == e.segment
                    && previous.first_bus <= e.last_bus
                    && e.first_bus <= previous.last_bus
                {
                    return Err(Error);
                }
                if previous.base + ((previous.first_bus as u64) << 20) < e.end()?
                    && e.base + ((e.first_bus as u64) << 20) < previous.end()?
                {
                    return Err(Error);
                }
            }
        }
        Ok(result)
    }
    pub fn regions(&self) -> impl Iterator<Item = Ecam> + '_ {
        self.bytes[44..].as_chunks::<16>().0.iter().map(|b| Ecam {
            base: u64::from_le_bytes(b[..8].try_into().expect("MCFG record")),
            segment: u16::from_le_bytes(b[8..10].try_into().expect("MCFG segment")),
            first_bus: b[10],
            last_bus: b[11],
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hpet {
    pub address: u64,
    pub number: u8,
    pub minimum_tick: u16,
}
impl Hpet {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        acpi::sdt(bytes, b"HPET")?;
        if bytes.len() != 56
            || bytes[40] != 0
            || bytes[41] != 64
            || bytes[42] != 0
            || !matches!(bytes[43], 0 | 4)
        {
            return Err(Error);
        }
        let address = u64_at(bytes, 44)?;
        if address == 0
            || address & 1023 != 0
            || address.checked_add(1024).is_none_or(|e| e > 1u64 << 52)
        {
            return Err(Error);
        }
        Ok(Self {
            address,
            number: bytes[52],
            minimum_tick: u16_at(bytes, 53)?,
        })
    }
}
pub struct Madt<'a> {
    bytes: &'a [u8],
}
impl<'a> Madt<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        acpi::sdt(bytes, b"APIC")?;
        if bytes.len() < 44 || u32_at(bytes, 40)? & !1 != 0 {
            return Err(Error);
        }
        let mut at = 44;
        while at < bytes.len() {
            let header = bytes.get(at..at + 2).ok_or(Error)?;
            let n = header[1] as usize;
            let min = match header[0] {
                0 => 8,
                1 => 12,
                2 => 10,
                3 => 8,
                4 => 6,
                5 => 12,
                9 => 16,
                10 => 12,
                _ => 2,
            };
            if n < min || at.checked_add(n).is_none_or(|end| end > bytes.len()) {
                return Err(Error);
            }
            at += n;
        }
        Ok(Self { bytes })
    }
    pub fn local_apic(&self) -> Result<u64, Error> {
        let mut base = u32_at(self.bytes, 36)? as u64;
        let mut found = false;
        for (kind, data) in self.entries() {
            if kind == 5 {
                if found || data[2..4] != [0; 2] {
                    return Err(Error);
                }
                base = u64_at(data, 4)?;
                found = true;
            }
        }
        if base == 0 || base & 4095 != 0 || base.checked_add(4096).is_none_or(|e| e > 1u64 << 52) {
            return Err(Error);
        }
        Ok(base)
    }
    pub fn entries(&self) -> impl Iterator<Item = (u8, &'a [u8])> + '_ {
        let mut at = 44;
        core::iter::from_fn(move || {
            if at == self.bytes.len() {
                return None;
            }
            let n = self.bytes[at + 1] as usize;
            let b = &self.bytes[at..at + n];
            at += n;
            Some((b[0], b))
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IommuUnit<'a> {
    pub kind: u8,
    pub flags: u8,
    pub segment: u16,
    pub device: u16,
    pub capability: u16,
    pub mmio: u64,
    pub info: u16,
    pub feature_attributes: u32,
    pub extended_features: Option<(u64, u64)>,
    entries: &'a [u8],
}
impl IommuUnit<'_> {
    pub fn requester_alias(&self, id: u16) -> Result<Option<u16>, Error> {
        let mut at = 0;
        let mut start = None;
        let mut answer = None;
        while at < self.entries.len() {
            let b = &self.entries[at..];
            let t = b[0];
            let n = entry_length(t)?;
            let devid = u16_at(b, 1)?;
            match t {
                0 => {}
                1 => {
                    answer = Some(id);
                }
                2 | 0x46 => {
                    if devid == id {
                        answer = Some(id);
                    }
                }
                3 | 0x47 => {
                    if start.is_some() {
                        return Err(Error);
                    }
                    start = Some((devid, None));
                }
                4 => {
                    let (first, alias) = start.take().ok_or(Error)?;
                    if first > devid {
                        return Err(Error);
                    }
                    if (first..=devid).contains(&id) {
                        answer = Some(alias.unwrap_or(id));
                    }
                }
                0x42 => {
                    if devid == id {
                        answer = Some(u16_at(b, 5)?);
                    }
                }
                0x43 => {
                    if start.is_some() {
                        return Err(Error);
                    }
                    start = Some((devid, Some(u16_at(b, 5)?)));
                }
                0x48 => {}
                _ => return Err(Error),
            }
            at += n;
        }
        if start.is_some() {
            return Err(Error);
        }
        Ok(answer)
    }
}
fn entry_length(kind: u8) -> Result<usize, Error> {
    match kind {
        0..=4 => Ok(4),
        0x42 | 0x43 | 0x46 | 0x47 | 0x48 => Ok(8),
        _ => Err(Error),
    }
}
pub struct Ivrs<'a> {
    bytes: &'a [u8],
    pub info: u32,
}
impl<'a> Ivrs<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        acpi::sdt(bytes, b"IVRS")?;
        if bytes.len() < 48 || bytes[40..48] != [0; 8] {
            return Err(Error);
        }
        let result = Self {
            bytes,
            info: u32_at(bytes, 36)?,
        };
        let mut at = 48;
        let mut count = 0;
        while at < bytes.len() {
            let h = bytes.get(at..at + 4).ok_or(Error)?;
            let n = u16_at(h, 2)? as usize;
            let fixed = match h[0] {
                0x10 => 24,
                0x11 => 40,
                _ => return Err(Error),
            };
            if n < fixed || at.checked_add(n).is_none_or(|e| e > bytes.len()) {
                return Err(Error);
            }
            let unit = parse_unit(&bytes[at..at + n])?;
            let mut p = 0;
            while p < unit.entries.len() {
                let k = unit.entries[p];
                let length = entry_length(k)?;
                if p + length > unit.entries.len() {
                    return Err(Error);
                }
                p += length;
            }
            // Validate range structure even when this block is an older duplicate.
            unit.requester_alias(0)?;
            for previous in result.raw_units().take(count) {
                if previous.segment == unit.segment && previous.device == unit.device {
                    if previous.kind == unit.kind
                        || previous.mmio != unit.mmio
                        || previous.capability != unit.capability
                        || previous.entries != unit.entries
                    {
                        return Err(Error);
                    }
                } else if previous.mmio == unit.mmio {
                    return Err(Error);
                }
            }
            count += 1;
            if count > 16 {
                return Err(Error);
            }
            at += n;
        }
        if count == 0 {
            return Err(Error);
        }
        Ok(result)
    }
    fn raw_units(&self) -> impl Iterator<Item = IommuUnit<'a>> + '_ {
        let mut at = 48;
        core::iter::from_fn(move || {
            if at >= self.bytes.len() {
                return None;
            }
            let n = u16_at(self.bytes, at + 2).expect("validated IVHD length") as usize;
            let u = parse_unit(&self.bytes[at..at + n]).expect("validated IVHD");
            at += n;
            Some(u)
        })
    }
    pub fn units(&self) -> impl Iterator<Item = IommuUnit<'a>> + '_ {
        self.raw_units().filter(|u| {
            u.kind == 0x11
                || !self.raw_units().any(|other| {
                    other.kind == 0x11 && other.segment == u.segment && other.device == u.device
                })
        })
    }
    pub fn unit_for(&self, segment: u16, requester: u16) -> Result<(IommuUnit<'a>, u16), Error> {
        let mut found = None;
        for u in self.units() {
            if u.segment != segment {
                continue;
            }
            if let Some(alias) = u.requester_alias(requester)? {
                if found.is_some() {
                    return Err(Error);
                }
                found = Some((u, alias));
            }
        }
        found.ok_or(Error)
    }
}
fn parse_unit(bytes: &[u8]) -> Result<IommuUnit<'_>, Error> {
    let fixed = match bytes[0] {
        0x10 => 24,
        0x11 => 40,
        _ => return Err(Error),
    };
    if bytes.len() < fixed {
        return Err(Error);
    }
    let mmio = u64_at(bytes, 8)?;
    let capability = u16_at(bytes, 6)?;
    if mmio == 0
        || mmio & 16383 != 0
        || mmio.checked_add(16384).is_none_or(|e| e > 1u64 << 52)
        || capability & 3 != 0
        || !(0x40..=0xfc).contains(&capability)
    {
        return Err(Error);
    }
    Ok(IommuUnit {
        kind: bytes[0],
        flags: bytes[1],
        device: u16_at(bytes, 4)?,
        capability,
        mmio,
        segment: u16_at(bytes, 16)?,
        info: u16_at(bytes, 18)?,
        feature_attributes: u32_at(bytes, 20)?,
        extended_features: if fixed == 40 {
            Some((u64_at(bytes, 24)?, u64_at(bytes, 32)?))
        } else {
            None
        },
        entries: &bytes[fixed..],
    })
}
