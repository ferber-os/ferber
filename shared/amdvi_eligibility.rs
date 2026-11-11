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
            0x08 => (0x1022, 0x1419, 0x08, 0x06, 0x00),
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
