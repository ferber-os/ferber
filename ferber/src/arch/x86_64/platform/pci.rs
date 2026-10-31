//! Side-effect-free PCI configuration decoding.
#![allow(dead_code)]
use super::acpi::{u16_at, Error};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Function {
    pub segment: u16,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}
impl Function {
    pub fn requester(&self) -> Result<u16, Error> {
        if self.device >= 32 || self.function >= 8 {
            return Err(Error);
        }
        Ok(((self.bus as u16) << 8) | ((self.device as u16) << 3) | self.function as u16)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub interface: u8,
    pub header: u8,
    pub multifunction: bool,
}
impl Identity {
    pub fn parse(bytes: &[u8]) -> Result<Option<Self>, Error> {
        if bytes.len() < 64 {
            return Err(Error);
        }
        let vendor = u16_at(bytes, 0)?;
        if vendor == 0xffff {
            return Ok(None);
        }
        if vendor == 0 {
            return Err(Error);
        }
        let header = bytes[14];
        if header & 0x7f > 2 {
            return Err(Error);
        }
        Ok(Some(Self {
            vendor,
            device: u16_at(bytes, 2)?,
            interface: bytes[9],
            subclass: bytes[10],
            class: bytes[11],
            header: header & 0x7f,
            multifunction: header & 0x80 != 0,
        }))
    }
    pub fn is_xhci(&self) -> bool {
        self.class == 0x0c && self.subclass == 3 && self.interface == 0x30 && self.header == 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBar {
    pub base: u64,
    pub length: u64,
    pub is_64: bool,
    pub prefetchable: bool,
}
impl MemoryBar {
    /// `length` comes from a separately authorized BAR-sizing operation.
    pub fn decode(low: u32, high: u32, length: u64) -> Result<Self, Error> {
        let kind = (low >> 1) & 3;
        if low & 1 != 0 || !matches!(kind, 0 | 2) || length < 16 || !length.is_power_of_two() {
            return Err(Error);
        }
        let base = (low as u64 & !15) | if kind == 2 { (high as u64) << 32 } else { 0 };
        if base == 0
            || base & (length - 1) != 0
            || base.checked_add(length).is_none_or(|e| e > 1u64 << 52)
        {
            return Err(Error);
        }
        Ok(Self {
            base,
            length,
            is_64: kind == 2,
            prefetchable: low & 8 != 0,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability {
    pub id: u8,
    pub offset: u8,
}
pub struct Capabilities<'a> {
    bytes: &'a [u8],
}
impl<'a> Capabilities<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 256 {
            return Err(Error);
        }
        let identity = Identity::parse(bytes)?.ok_or(Error)?;
        if identity.header != 0 {
            return Err(Error);
        }
        let mut seen = [false; 64];
        let mut at = if u16_at(bytes, 6)? & 0x10 != 0 {
            bytes[0x34]
        } else {
            0
        };
        while at != 0 {
            if at < 0x40 || at & 3 != 0 || at > 0xfc || seen[at as usize / 4] {
                return Err(Error);
            }
            seen[at as usize / 4] = true;
            let size = match bytes[at as usize] {
                5 => {
                    if u16_at(bytes, at as usize + 2)? & 0x80 != 0 {
                        14
                    } else {
                        10
                    }
                }
                0x11 => 12,
                _ => 2,
            };
            if at as usize + size > 256 {
                return Err(Error);
            }
            at = bytes[at as usize + 1];
        }
        Ok(Self { bytes })
    }
    pub fn entries(&self) -> impl Iterator<Item = Capability> + '_ {
        let mut at = if u16_at(self.bytes, 6).expect("validated PCI status") & 0x10 != 0 {
            self.bytes[0x34]
        } else {
            0
        };
        core::iter::from_fn(move || {
            if at == 0 {
                return None;
            }
            let out = Capability {
                id: self.bytes[at as usize],
                offset: at,
            };
            at = self.bytes[at as usize + 1];
            Some(out)
        })
    }
}
