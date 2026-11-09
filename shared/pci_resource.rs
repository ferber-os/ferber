//! Pointer-free firmware resource bounds for the diagnostic xHCI handoff.
#![allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControllerResource {
    pub version: u32,
    pub flags: u32,
    pub base: u64,
    pub length: u64,
    pub raw_bar: u64,
    pub segment: u16,
    pub vendor: u16,
    pub device: u16,
    pub command_snapshot: u16,
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub bar_index: u8,
    pub class_revision: u32,
    pub reserved: [u64; 2],
}
impl ControllerResource {
    pub const NONE: Self = Self {
        version: 0,
        flags: 0,
        base: 0,
        length: 0,
        raw_bar: 0,
        segment: 0,
        vendor: 0,
        device: 0,
        command_snapshot: 0,
        bus: 0,
        slot: 0,
        function: 0,
        bar_index: 0,
        class_revision: 0,
        reserved: [0; 2],
    };
    pub fn validate(&self) -> Result<Option<(u64, u64)>, Error> {
        if *self == Self::NONE {
            return Ok(None);
        }
        let kind = (self.raw_bar >> 1) & 3;
        if self.version != 1
            || self.flags != 0
            || self.reserved != [0; 2]
            || self.slot >= 32
            || self.function >= 8
            || self.bar_index != 0
            || self.vendor == 0
            || self.vendor == 0xffff
            || self.class_revision >> 8 != 0x0c0330
            || self.raw_bar & 1 != 0
            || !matches!(kind, 0 | 2)
            || kind == 0 && self.raw_bar >> 32 != 0
            || self.base != self.raw_bar & !15
            || self.base == 0
            || self.length < 4096
            || self.length > 16 * 1024 * 1024
            || !self.length.is_power_of_two()
            || self.base & (self.length - 1) != 0
            || self
                .base
                .checked_add(self.length)
                .is_none_or(|end| end > 1u64 << 52)
        {
            return Err(Error);
        }
        Ok(Some((self.base, self.length)))
    }
}
pub const fn selected_identity(msi: bool) -> (u16, u16) {
    if msi {
        (0x1022, 0x15b8)
    } else {
        (0x1b36, 0x000d)
    }
}
/// One ACPI QWORD memory resource and its EndTag, as returned by GetBarAttributes.
pub fn descriptor(bytes: &[u8], raw_bar: u64) -> Result<(u64, u64), Error> {
    if bytes.len() != 48
        || bytes[0] != 0x8a
        || bytes[1..3] != [43, 0]
        || bytes[3] != 0
        || bytes[46] != 0x79
    {
        return Err(Error);
    }
    let word = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let base = word(14);
    let length = word(38);
    // Firmware reports either the inclusive end or EDK2's BAR alignment mask.
    if !matches!(word(6), 32 | 64)
        || word(30) != 0
        || length == 0
        || (word(22) != length - 1 && base.checked_add(length - 1) != Some(word(22)))
        || base.checked_add(length).is_none()
        || base != raw_bar & !15
        || bytes[47] != 0 && bytes.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0
    {
        return Err(Error);
    }
    Ok((base, length))
}
const _: () = assert!(core::mem::size_of::<ControllerResource>() == 64);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_msi_descriptor_accepts_inclusive_end_and_rejects_other_maxima() {
        let words: [u64; 6] = [0x0040000000002b8a, 0, 0xffff00000000f6a0,
            0x000000000000f6af, 0, 0x0079000000000010];
        let mut bytes = [0; 48];
        for (row, word) in bytes.chunks_exact_mut(8).zip(words) {
            row.copy_from_slice(&word.to_le_bytes());
        }
        assert_eq!(descriptor(&bytes, 0xf6a00004), Ok((0xf6a00000, 0x100000)));
        for maximum in [0xfffffu64, 0xf6afffff] {
            bytes[22..30].copy_from_slice(&maximum.to_le_bytes());
            assert!(descriptor(&bytes, 0xf6a00004).is_ok());
        }
        for maximum in [0xffffe, 0xf6affffe, 0xf6b00000, u64::MAX] {
            bytes[22..30].copy_from_slice(&maximum.to_le_bytes());
            assert!(descriptor(&bytes, 0xf6a00004).is_err());
        }
        bytes[22..30].copy_from_slice(&0xf6afffffu64.to_le_bytes());
        assert!(descriptor(&bytes, 0xf6b00004).is_err());
        let resource = ControllerResource { base: 0xf6a00000, length: 0x100000,
            raw_bar: 0xf6a00004, ..record() };
        assert_eq!(resource.validate(), Ok(Some((0xf6a00000, 0x100000))));
    }
    fn record() -> ControllerResource {
        ControllerResource {
            version: 1,
            base: 0xf000_0000,
            length: 0x4000,
            raw_bar: 0xf000_0004,
            vendor: 0x1b36,
            device: 0x000d,
            class_revision: 0x0c033000,
            ..ControllerResource::NONE
        }
    }
    fn descriptor_bytes() -> [u8; 48] {
        let r = record();
        let mut b = [0; 48];
        b[0] = 0x8a;
        b[1] = 43;
        b[6..14].copy_from_slice(&64u64.to_le_bytes());
        b[14..22].copy_from_slice(&r.base.to_le_bytes());
        b[22..30].copy_from_slice(&(r.length - 1).to_le_bytes());
        b[38..46].copy_from_slice(&r.length.to_le_bytes());
        b[46] = 0x79;
        b
    }
    #[test]
    fn optional_record_is_strict_and_bounded() {
        assert_eq!(ControllerResource::NONE.validate(), Ok(None));
        let r = record();
        assert_eq!(r.validate(), Ok(Some((r.base, r.length))));
        for mutate in [
            |r: &mut ControllerResource| r.version = 0,
            |r: &mut ControllerResource| r.reserved[0] = 1,
            |r: &mut ControllerResource| r.flags = 1,
            |r: &mut ControllerResource| r.raw_bar |= 1,
            |r: &mut ControllerResource| r.slot = 32,
            |r: &mut ControllerResource| r.length = 32 * 1024 * 1024,
            |r: &mut ControllerResource| r.base += 4096,
            |r: &mut ControllerResource| r.class_revision = 0,
        ] {
            let mut bad = r;
            mutate(&mut bad);
            assert!(bad.validate().is_err());
        }
    }
    #[test]
    fn descriptors_require_exact_bounded_memory_resource() {
        let r = record();
        let b = descriptor_bytes();
        assert_eq!(descriptor(&b, r.raw_bar), Ok((r.base, r.length)));
        for n in 0..48 {
            assert!(descriptor(&b[..n], r.raw_bar).is_err());
        }
        for offset in [0, 1, 2, 3, 6, 14, 22, 30, 38, 46, 47] {
            let mut bad = b;
            bad[offset] ^= 1;
            assert!(descriptor(&bad, r.raw_bar).is_err(), "offset {offset}");
        }
        assert!(descriptor(&b, r.raw_bar + 0x1000).is_err());
    }
}
