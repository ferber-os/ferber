//! Boot-only supervisor UC mappings for validated firmware/device apertures.
#![allow(dead_code)]
use crate::boot_info::{BootInfo, MEMORY_RAM_RESERVED, MEMORY_USABLE};
use alloc::vec::Vec;
#[path = "../../../../../shared/platform_mmio.rs"]
pub mod bounds;
#[derive(Clone, Copy, Debug)]
pub struct MappedRegion {
    base: usize,
    length: usize,
    physical: u64,
}
impl MappedRegion {
    pub fn physical(&self) -> u64 {
        self.physical
    }
    pub fn virtual_base(&self) -> usize {
        self.base
    }
    pub fn length(&self) -> usize {
        self.length
    }
    pub fn subregion(&self, offset: usize, length: usize) -> Result<Self, bounds::Error> {
        self.check(offset, length, 1)?;
        Ok(Self {
            base: self.base + offset,
            length,
            physical: self.physical + offset as u64,
        })
    }
    fn check(&self, offset: usize, width: usize, alignment: usize) -> Result<(), bounds::Error> {
        if self
            .base
            .checked_add(offset)
            .is_none_or(|address| address & (alignment - 1) != 0)
            || width == 0
            || offset
                .checked_add(width)
                .is_none_or(|end| end > self.length)
        {
            Err(bounds::Error)
        } else {
            Ok(())
        }
    }
    pub fn read32(&self, offset: usize) -> Result<u32, bounds::Error> {
        self.check(offset, 4, 4)?;
        Ok(unsafe { core::ptr::read_volatile((self.base + offset) as *const u32) })
    }
    pub fn read64(&self, offset: usize) -> Result<u64, bounds::Error> {
        self.check(offset, 8, 8)?;
        Ok(unsafe { core::ptr::read_volatile((self.base + offset) as *const u64) })
    }
    /// # Safety
    /// Caller owns the device register and implements its required protocol.
    pub unsafe fn write32(&self, offset: usize, value: u32) -> Result<(), bounds::Error> {
        self.check(offset, 4, 4)?;
        core::ptr::write_volatile((self.base + offset) as *mut u32, value);
        Ok(())
    }
    /// # Safety
    /// Caller owns the device register and implements its required protocol.
    pub unsafe fn write64(&self, offset: usize, value: u64) -> Result<(), bounds::Error> {
        self.check(offset, 8, 8)?;
        core::ptr::write_volatile((self.base + offset) as *mut u64, value);
        Ok(())
    }
}
pub struct MmioMapper {
    forbidden: Vec<(u64, u64)>,
    mapped: Vec<MappedRegion>,
    next: usize,
    bits: u32,
}
impl MmioMapper {
    /// # Safety
    /// Bootstrap CPU, IF=0, permanent kernel root active, before any user roots
    /// exist. `info` and its map remain readable throughout this call.
    pub unsafe fn new(info: &BootInfo) -> Result<Self, bounds::Error> {
        info.validate().map_err(|_| bounds::Error)?;
        let mut forbidden = Vec::new();
        for r in info.regions() {
            if matches!(r.kind, MEMORY_USABLE | MEMORY_RAM_RESERVED) {
                forbidden.push((r.start, r.end));
            }
        }
        forbidden.push((info.kernel_start, info.kernel_end));
        forbidden.push((
            info.boot_info_physical,
            info.boot_info_physical + core::mem::size_of::<BootInfo>() as u64,
        ));
        forbidden.push((
            info.memory_regions,
            info.memory_regions + info.memory_region_count as u64 * info.memory_region_size as u64,
        ));
        if let Some(r) = info.acpi.bounds().map_err(|_| bounds::Error)? {
            forbidden.push(r);
        }
        if let Some(r) = info.boot_payload.bounds().map_err(|_| bounds::Error)? {
            forbidden.push(r);
        }
        if info.framebuffer.size != 0 {
            forbidden.push((
                info.framebuffer.base,
                info.framebuffer
                    .base
                    .checked_add(info.framebuffer.size)
                    .ok_or(bounds::Error)?,
            ));
        }
        let max = core::arch::x86_64::__cpuid(0x80000000).eax;
        let bits = if max >= 0x80000008 {
            core::arch::x86_64::__cpuid(0x80000008).eax & 255
        } else {
            36
        };
        Ok(Self {
            forbidden,
            mapped: Vec::new(),
            next: 0xffff_fffe_1000_0000,
            bits: bits.min(52),
        })
    }
    /// # Safety
    /// Caller proves this complete aperture belongs to a discovered ACPI table
    /// or a sized, assigned PCI BAR. Arbitrary physical addresses are forbidden.
    /// Boot CPU only, permanent root active, no user process roots created yet.
    pub unsafe fn map(
        &mut self,
        physical: u64,
        length: u64,
    ) -> Result<MappedRegion, bounds::Error> {
        self.map_with(physical, length, |va, pa, bytes| {
            crate::arch::paging::map_platform_mmio(va, pa, bytes).map_err(|_| bounds::Error)
        })
    }

    fn map_with(&mut self, physical: u64, length: u64,
        install: impl FnOnce(usize, usize, usize) -> Result<(), bounds::Error>,
    ) -> Result<MappedRegion, bounds::Error> {
        let (start, end) = bounds::validate(physical, length, self.bits, &self.forbidden)?;
        for r in &self.mapped {
            let mapped_end = r.physical + r.length as u64;
            if r.physical <= start && end <= mapped_end {
                return r.subregion((physical - r.physical) as usize, length as usize);
            }
            if start < mapped_end && r.physical < end {
                return Err(bounds::Error);
            }
        }
        let bytes = (end - start) as usize;
        let next = self.next.checked_add(bytes).ok_or(bounds::Error)?;
        if next > 0xffff_fffe_7000_0000 {
            return Err(bounds::Error);
        }
        self.mapped.try_reserve(1).map_err(|_| bounds::Error)?;
        let base = self.next;
        // Failed permanent mappings can retain a prefix. Burn its entire VA span
        // before publication so neither retries nor other devices can reuse it.
        self.next = next;
        install(base, start as usize, bytes)?;
        let r = MappedRegion {
            base,
            length: bytes,
            physical: start,
        };
        self.mapped.push(r);
        r.subregion((physical - start) as usize, length as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_partial_mapping_quarantines_entire_span_before_retry() {
        let base = 0xffff_fffe_1000_0000;
        let mut mapper = MmioMapper { forbidden: Vec::new(), mapped: Vec::new(), next: base, bits: 48 };
        let mut published = Vec::new();
        assert!(mapper.map_with(0xf0000000, 8192, |va, _, _| {
            published.push(va); // First leaf survived a later table-allocation failure.
            Err(bounds::Error)
        }).is_err());
        assert_eq!(mapper.next, base + 8192);
        assert!(mapper.mapped.is_empty());
        let region = mapper.map_with(0xf0000000, 8192, |va, _, _| {
            assert!(!published.contains(&va), "retry reused a partially published leaf");
            published.push(va);
            Ok(())
        }).unwrap();
        assert_eq!(region.virtual_base(), base + 8192);
        assert_eq!(mapper.next, base + 16384);
        mapper.map_with(0xf0000100, 16, |_, _, _| panic!("successful mapping was not reused")).unwrap();
        assert_eq!(mapper.next, base + 16384);
    }
}
