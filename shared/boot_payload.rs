//! Bounded external ELF source, reserved independently of allocator-managed RAM.
pub const PAYLOAD_BASE: u64 = 0xffff_fffe_0100_0000;
pub const MAX_PAYLOAD_BYTES: u64 = 4 * 1024 * 1024;
pub const PAYLOAD_WINDOW_BYTES: u64 = 8 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BootPayload {
    pub physical_start: u64,
    pub byte_len: u64,
    pub virtual_start: u64,
}

impl BootPayload {
    pub const NONE: Self = Self {
        physical_start: 0,
        byte_len: 0,
        virtual_start: 0,
    };

    pub fn new(physical_start: u64, byte_len: u64) -> Result<Self, &'static str> {
        let result = Self {
            physical_start,
            byte_len,
            virtual_start: PAYLOAD_BASE + (physical_start & 4095),
        };
        result.bounds()?;
        Ok(result)
    }

    /// Validate the descriptor and return its page-rounded backing, if present.
    pub fn bounds(self) -> Result<Option<(u64, u64)>, &'static str> {
        if self == Self::NONE {
            return Ok(None);
        }
        if self.physical_start < 4096 || self.byte_len == 0 || self.byte_len > MAX_PAYLOAD_BYTES {
            return Err("invalid boot payload length");
        }
        let end = self
            .physical_start
            .checked_add(self.byte_len)
            .and_then(|end| end.checked_add(4095))
            .ok_or("boot payload range overflow")?
            & !4095;
        let start = self.physical_start & !4095;
        if self.virtual_start != PAYLOAD_BASE + (self.physical_start & 4095)
            || end - start > PAYLOAD_WINDOW_BYTES
        {
            return Err("invalid boot payload alias");
        }
        Ok(Some((start, end)))
    }
}

const _: () = assert!(core::mem::size_of::<BootPayload>() == 24);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_absence_rounding_length_and_alias() {
        assert_eq!(BootPayload::NONE.bounds(), Ok(None));
        let mut p = BootPayload::new(0x2345, 4096).unwrap();
        assert_eq!(p.bounds(), Ok(Some((0x2000, 0x4000))));
        p.virtual_start += 1;
        assert!(p.bounds().is_err());
        for (start, len) in [
            (0, 1),
            (1, 1),
            (1, 0),
            (1, MAX_PAYLOAD_BYTES + 1),
            (u64::MAX, 1),
        ] {
            assert!(BootPayload::new(start, len).is_err());
        }
        assert!(BootPayload {
            byte_len: 1,
            ..BootPayload::NONE
        }
        .bounds()
        .is_err());
    }
}
