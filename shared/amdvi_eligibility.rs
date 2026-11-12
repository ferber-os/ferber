//! Eligibility for one controlled QEMU DMA diagnostic, never physical takeover.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub unit: u16,
    pub alias: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub segment: u16,
    pub requester: u16,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub interface: u8,
    pub header: u8,
    pub coverage: Option<Coverage>,
    /// Result of the existing full PCI extended-capability validation.
    pub extensions_valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    FunctionCount,
    DuplicateRequester(u16),
    Topology(u16),
    Extensions(u16),
    MissingCoverage(u16),
    WrongUnit(u16),
    SharedAlias(u16),
    UnsupportedAlias(u16),
    UnitCapabilities,
}

/// QEMU's AMD-Vi exposes a four-byte placeholder MSI capability followed by HT
/// and the real MSI capability. Do not accept this overlap in the generic parser.
/// Only the known QEMU 8.2 zero ID and QEMU 11.1.3 ID are accepted here.
pub fn qemu_iommu_msi_offset(bytes: &[u8; 256]) -> Result<usize, Error> {
    let word = |at| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    if word(0) != 0x1022
        || !matches!(word(2), 0x0000 | 0x1419)
        || bytes[9..12] != [0, 6, 8]
        || bytes[14] != 0
        || word(6) & 0x10 == 0
        || bytes[0x34] != 0x60
        || bytes[0x40..0x42] != [0x0f, 0]
        || bytes[0x58..0x5a] != [0x05, 0x40]
        || word(0x5a) != 0
        || bytes[0x5c..0x5e] != [0x08, 0x58]
        || bytes[0x60..0x62] != [0x05, 0x5c]
        || word(0x62) & !0x81 != 0
        || word(0x62) & 0x80 == 0
    {
        return Err(Error::UnitCapabilities);
    }
    Ok(0x60)
}

/// Exact Q35 profile with AMD-Vi, EDU at04.0 and standard VGA at02.0 for GOP.
/// This checks topology, not device idleness, interrupt masking or DMA draining.
/// The caller must separately establish those facts before activation.
pub fn qemu_edu(entries: &[Entry]) -> Result<(), Error> {
    if entries.len() != 7 {
        return Err(Error::FunctionCount);
    }
    for (index, entry) in entries.iter().enumerate() {
        let rid = entry.requester;
        if entries[..index]
            .iter()
            .any(|previous| previous.segment == entry.segment && previous.requester == rid)
        {
            return Err(Error::DuplicateRequester(rid));
        }
        // Reject bridge paths; same-bus routing still relies on the QEMU model contract.
        let expected = match rid {
            0x00 => (0x8086, 0x29c0, 0x06, 0x00, 0x00),
            // QEMU 8.2 left the ID zero; QEMU 11.1.3 advertises 1419. Model-only.
            0x08 if matches!(entry.device, 0x0000 | 0x1419) => {
                (0x1022, entry.device, 0x08, 0x06, 0x00)
            }
            0x10 => (0x1234, 0x1111, 0x03, 0x00, 0x00),
            0x20 => (0x1234, 0x11e8, 0x00, 0xff, 0x00),
            0xf8 => (0x8086, 0x2918, 0x06, 0x01, 0x00),
            0xfa => (0x8086, 0x2922, 0x01, 0x06, 0x01),
            0xfb => (0x8086, 0x2930, 0x0c, 0x05, 0x00),
            _ => return Err(Error::Topology(rid)),
        };
        if entry.segment != 0
            || entry.header != 0
            || (
                entry.vendor,
                entry.device,
                entry.class,
                entry.subclass,
                entry.interface,
            ) != expected
        {
            return Err(Error::Topology(rid));
        }
        if !entry.extensions_valid {
            return Err(Error::Extensions(rid));
        }
        let Some(coverage) = entry.coverage else {
            // Only these exact QEMU structural functions may be outside IVRS.
            if matches!(rid, 0x00 | 0x08) {
                continue;
            }
            return Err(Error::MissingCoverage(rid));
        };
        if coverage.unit != 0x08 {
            return Err(Error::WrongUnit(rid));
        }
        if entries[..index].iter().any(|previous| {
            previous.coverage.is_some_and(|other| {
                previous.segment == entry.segment
                    && other.unit == coverage.unit
                    && other.alias == coverage.alias
            })
        }) {
            return Err(Error::SharedAlias(coverage.alias));
        }
        if coverage.alias != rid {
            return Err(Error::UnsupportedAlias(rid));
        }
    }
    Ok(())
}
