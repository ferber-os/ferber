//! Read-only PCI observations. These values never authorize physical DMA takeover.
#![allow(dead_code)]
use super::pci::{Capabilities, Identity};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Read,
    Chain,
    Duplicate(u16),
    Version(u16),
    Bridge,
    Parent,
    Ambiguous,
    Depth,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Acs {
    pub capability: u16,
    pub control: u16,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extensions {
    pub ats: Option<bool>,
    pub pri: Option<bool>,
    pub pasid: Option<bool>,
    pub acs: Option<Acs>,
}
/// Counts known observations without turning a failed decode into absence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtensionSummary {
    pub conventional_only: usize,
    pub known_enabled: usize,
    pub unknown: usize,
}
impl ExtensionSummary {
    pub fn record(&mut self, observation: Result<Extensions, Error>) {
        match observation {
            Ok(value)
                if value.ats == Some(true)
                    || value.pri == Some(true)
                    || value.pasid == Some(true) =>
            {
                self.known_enabled += 1
            }
            Ok(_) => {}
            Err(_) => self.unknown += 1,
        }
    }
    pub fn complete(&self) -> bool {
        self.unknown == 0
    }
}
/// Original reads retained for diagnostics; reporting never rereads configuration.
#[derive(Clone, Copy, Debug)]
pub struct ExtensionSnapshot {
    pub space: ConfigSpace,
    pub result: Result<Extensions, Error>,
    pub raw100: Option<Result<u32, Error>>,
    pub last_read: Option<(usize, Result<u32, Error>)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigSpace {
    Unknown,
    ConventionalOnly,
    Extended,
}
/// Conventional metadata determines whether an extended-capability read applies.
pub fn capture_configuration(
    bytes: Result<[u8; 256], Error>,
    read: impl FnMut(usize) -> Result<u32, Error>,
) -> ExtensionSnapshot {
    let space = bytes.and_then(|bytes| configuration_space(&bytes));
    match space {
        Ok(ConfigSpace::Extended) => capture_extensions(read),
        other => ExtensionSnapshot {
            space: other.unwrap_or(ConfigSpace::Unknown),
            result: other.map(|_| Extensions::default()),
            raw100: None,
            last_read: None,
        },
    }
}
fn configuration_space(bytes: &[u8; 256]) -> Result<ConfigSpace, Error> {
    let identity = Identity::parse(bytes)
        .map_err(|_| Error::Chain)?
        .ok_or(Error::Chain)?;
    let caps = Capabilities::parse(bytes).map_err(|_| Error::Chain)?;
    let mut extended = identity.class == 6 && identity.subclass == 0;
    for cap in caps.entries() {
        if cap.id == 0x10 {
            extended = true;
        } else if cap.id == 7 {
            let at = cap.offset as usize;
            // PCI-X status occupies bytes 4..8; do not consume another capability.
            if at + 8 > bytes.len()
                || caps
                    .entries()
                    .any(|next| next.offset > cap.offset && (next.offset as usize) < at + 8)
            {
                return Err(Error::Chain);
            }
            let status = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
            extended |= status & ((1 << 30) | (1 << 31)) != 0;
        }
    }
    Ok(if extended {
        ConfigSpace::Extended
    } else {
        ConfigSpace::ConventionalOnly
    })
}
pub fn capture_extensions(mut read: impl FnMut(usize) -> Result<u32, Error>) -> ExtensionSnapshot {
    let mut raw100 = None;
    let mut last_read = None;
    let result = extensions(|at| {
        let value = read(at);
        if at == 0x100 {
            raw100 = Some(value);
        }
        last_read = Some((at, value));
        value
    });
    ExtensionSnapshot {
        space: ConfigSpace::Extended,
        result,
        raw100,
        last_read,
    }
}
/// Observe enable bits without rejecting enabled devices or changing configuration.
pub fn extensions(mut read: impl FnMut(usize) -> Result<u32, Error>) -> Result<Extensions, Error> {
    let mut result = Extensions::default();
    let mut seen = [false; 1024];
    let mut at = 0x100;
    loop {
        if !(0x100..=0xffc).contains(&at) || at & 3 != 0 || seen[at / 4] {
            return Err(Error::Chain);
        }
        let header = read(at)?;
        if header == 0 || header == u32::MAX {
            return if at == 0x100 {
                Ok(result)
            } else {
                Err(Error::Chain)
            };
        }
        let id = header as u16;
        let version = (header >> 16) & 15;
        let known = matches!(id, 0x0d | 0x0f | 0x13 | 0x1b);
        if version == 0 || known && version != 1 {
            return Err(Error::Version(id));
        }
        let fixed = match id {
            0x13 => 16,
            _ if known => 8,
            _ => 4,
        };
        let mut end = at + fixed;
        if end > 4096 || seen[at / 4..end / 4].iter().any(|b| *b) {
            return Err(Error::Chain);
        }
        let value = if known { read(at + 4)? } else { 0 };
        if id == 0x0d && value & (1 << 5) != 0 {
            // ACS EC vector size zero encodes 256 bits; entries occupy whole DWORDs.
            let bits = match (value >> 8) & 255 {
                0 => 256,
                bits => bits as usize,
            };
            end += bits.div_ceil(32) * 4;
        }
        if end > 4096 || seen[at / 4..end / 4].iter().any(|b| *b) {
            return Err(Error::Chain);
        }
        seen[at / 4..end / 4].fill(true);
        if known {
            match id {
                0x0d => {
                    if result.acs.is_some() {
                        return Err(Error::Duplicate(id));
                    }
                    result.acs = Some(Acs {
                        capability: value as u16,
                        control: (value >> 16) as u16,
                    });
                }
                _ => {
                    let (slot, mask) = match id {
                        0x0f => (&mut result.ats, 1 << 31),
                        0x13 => (&mut result.pri, 1),
                        _ => (&mut result.pasid, 1 << 16),
                    };
                    if slot.replace(value & mask != 0).is_some() {
                        return Err(Error::Duplicate(id));
                    }
                }
            }
        }
        let next = (header >> 20) as usize;
        if next == 0 {
            return Ok(result);
        }
        at = next;
    }
}
#[derive(Clone, Copy)]
pub struct Function {
    pub segment: u16,
    pub requester: u16,
    pub header: u8,
    pub primary: u8,
    pub secondary: u8,
    pub subordinate: u8,
}
pub struct Path {
    pub bridges: [u16; 256],
    pub count: usize,
}
/// A bounded routing snapshot, not proof of ACS coverage or absence of bypass paths.
pub fn ancestry(
    functions: &[Function],
    segment: u16,
    requester: u16,
    roots: &[u8],
) -> Result<Path, Error> {
    let mut result = Path {
        bridges: [0; 256],
        count: 0,
    };
    let target_bus = (requester >> 8) as u8;
    let mut bus = target_bus;
    loop {
        let mut parent = None;
        for f in functions
            .iter()
            .filter(|f| f.segment == segment && f.header == 1)
        {
            if f.primary != (f.requester >> 8) as u8
                || f.secondary == 0
                || f.secondary > f.subordinate
                || f.primary >= f.secondary
            {
                return Err(Error::Bridge);
            }
            if f.secondary == bus && parent.replace(f).is_some() {
                return Err(Error::Ambiguous);
            }
        }
        match parent {
            Some(parent) => {
                if target_bus < parent.secondary || target_bus > parent.subordinate {
                    return Err(Error::Bridge);
                }
                if result.count == result.bridges.len() {
                    return Err(Error::Depth);
                }
                result.bridges[result.count] = parent.requester;
                result.count += 1;
                bus = parent.primary;
            }
            None if roots.contains(&bus) => return Ok(result),
            None => return Err(Error::Parent),
        }
    }
}
