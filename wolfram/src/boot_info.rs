//! Versioned, architecture-neutral handoff from firmware-facing boot code.
//!
//! Physical reservation addresses and readable virtual pointers are separate.
//! The provider retains the handoff and memory map through memory initialization.

pub const BOOT_INFO_MAGIC: u64 = 0x574f_4c46_5241_4d21; // "WOLFRAM!"
pub const BOOT_INFO_VERSION: u32 = 2;
#[cfg(target_arch = "x86_64")]
pub const BOOT_DATA_BASE: u64 = 0xffff_fffe_0000_0000;
#[cfg(target_arch = "x86_64")]
pub const BOOT_INFO_VA: u64 = BOOT_DATA_BASE;
#[cfg(target_arch = "x86_64")]
pub const BOOT_REGIONS_VA: u64 = BOOT_DATA_BASE + 0x1_0000;
#[cfg(target_arch = "x86_64")]
pub const MMIO_BASE: u64 = 0xffff_fffe_8000_0000;
pub const KERNEL_IMAGE_BASE: u64 = 0xffff_ffff_8000_0000;

pub const MEMORY_USABLE: u32 = 1;
pub const MEMORY_RESERVED: u32 = 2;

pub const PIXEL_FORMAT_NONE: u32 = 0;
#[cfg(target_arch = "x86_64")]
pub const PIXEL_FORMAT_RGB: u32 = 1;
#[cfg(target_arch = "x86_64")]
pub const PIXEL_FORMAT_BGR: u32 = 2;

/// A half-open physical address interval. Reserved intervals take precedence.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryRegion {
    pub start: u64,
    pub end: u64,
    pub kind: u32,
    pub reserved: u32,
}

impl MemoryRegion {
    pub const EMPTY: Self = Self {
        start: 0,
        end: 0,
        kind: MEMORY_RESERVED,
        reserved: 0,
    };

    pub const fn usable(start: u64, end: u64) -> Self {
        Self {
            start,
            end,
            kind: MEMORY_USABLE,
            reserved: 0,
        }
    }

    pub const fn reserved(start: u64, end: u64) -> Self {
        Self {
            start,
            end,
            kind: MEMORY_RESERVED,
            reserved: 0,
        }
    }
}

/// Optional framebuffer; `base == 0` or `size == 0` means absent.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Framebuffer {
    pub base: u64,
    pub virtual_base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32, // Pixels per scanline.
    pub pixel_format: u32,
}

impl Framebuffer {
    pub const NONE: Self = Self {
        base: 0,
        virtual_base: 0,
        size: 0,
        width: 0,
        height: 0,
        stride: 0,
        pixel_format: PIXEL_FORMAT_NONE,
    };

    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        self.size != 0
            && self
                .base
                .checked_add(self.size)
                .and_then(|end| end.checked_add(4095))
                .is_none_or(|fb_end| (self.base & !4095) < end && start < (fb_end & !4095))
    }
}

/// Binary contract shared by the UEFI loader and both kernel architectures.
/// The x86-64 entry signature is `extern "C" fn(*const BootInfo) -> !`.
#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub size: u32,
    pub memory_regions: u64,
    pub memory_region_count: u32,
    pub memory_region_size: u32,
    pub kernel_start: u64,
    pub kernel_end: u64,
    pub kernel_virtual_start: u64,
    pub boot_info_physical: u64,
    pub memory_regions_virtual: u64,
    pub framebuffer: Framebuffer,
}

impl BootInfo {
    pub fn new(
        regions: &[MemoryRegion],
        kernel_start: u64,
        kernel_end: u64,
        kernel_virtual_start: u64,
        boot_info_physical: u64,
        memory_regions_virtual: u64,
    ) -> Self {
        Self {
            magic: BOOT_INFO_MAGIC,
            version: BOOT_INFO_VERSION,
            size: core::mem::size_of::<Self>() as u32,
            memory_regions: regions.as_ptr() as u64,
            memory_region_count: regions.len() as u32,
            memory_region_size: core::mem::size_of::<MemoryRegion>() as u32,
            kernel_start,
            kernel_end,
            kernel_virtual_start,
            boot_info_physical,
            memory_regions_virtual,
            framebuffer: Framebuffer::NONE,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.magic != BOOT_INFO_MAGIC || self.version != BOOT_INFO_VERSION {
            return Err("unsupported boot information");
        }
        if self.size as usize != core::mem::size_of::<Self>()
            || self.memory_region_size as usize != core::mem::size_of::<MemoryRegion>()
        {
            return Err("invalid boot information layout");
        }
        if self.memory_region_count == 0 || self.memory_region_count > 4096 {
            return Err("invalid boot memory map count");
        }
        let bytes = (self.memory_region_count as usize)
            .checked_mul(core::mem::size_of::<MemoryRegion>())
            .ok_or("boot memory map overflow")?;
        if self.memory_regions == 0
            || !(self.memory_regions as usize).is_multiple_of(core::mem::align_of::<MemoryRegion>())
            || (self.memory_regions as usize).checked_add(bytes).is_none()
        {
            return Err("invalid boot memory map address");
        }
        if self.boot_info_physical == 0
            || self.memory_regions_virtual == 0
            || self.kernel_virtual_start == 0
        {
            return Err("missing version 2 boot addresses");
        }
        if !self
            .memory_regions_virtual
            .is_multiple_of(core::mem::align_of::<MemoryRegion>() as u64)
            || self
                .memory_regions_virtual
                .checked_add(bytes as u64)
                .is_none()
            || !self
                .boot_info_physical
                .is_multiple_of(core::mem::align_of::<Self>() as u64)
            || self
                .boot_info_physical
                .checked_add(core::mem::size_of::<Self>() as u64)
                .is_none()
        {
            return Err("invalid version 2 boot addresses");
        }
        if self.kernel_start >= self.kernel_end
            || self
                .kernel_virtual_start
                .checked_add(self.kernel_end - self.kernel_start)
                .is_none()
        {
            return Err("invalid kernel image range");
        }
        if self
            .framebuffer
            .overlaps(self.kernel_start, self.kernel_end)
            || self.framebuffer.overlaps(
                self.boot_info_physical,
                self.boot_info_physical + core::mem::size_of::<Self>() as u64,
            )
            || self
                .framebuffer
                .overlaps(self.memory_regions, self.memory_regions + bytes as u64)
            || (self.framebuffer.size != 0
                && (self.framebuffer.virtual_base == 0
                    || self
                        .framebuffer
                        .virtual_base
                        .checked_add(self.framebuffer.size)
                        .is_none()))
        {
            return Err("invalid framebuffer alias");
        }
        Ok(())
    }

    /// # Safety
    /// The boot provider must keep the virtual region array readable
    /// for `memory_region_count` entries through allocator initialization.
    pub unsafe fn regions(&self) -> &[MemoryRegion] {
        core::slice::from_raw_parts(
            self.memory_regions_virtual as *const MemoryRegion,
            self.memory_region_count as usize,
        )
    }
}

const _: () = assert!(core::mem::size_of::<MemoryRegion>() == 24);
const _: () = assert!(core::mem::size_of::<Framebuffer>() == 40);
const _: () = assert!(core::mem::size_of::<BootInfo>() == 112);
const _: () = assert!(core::mem::offset_of!(BootInfo, kernel_virtual_start) == 48);
const _: () = assert!(core::mem::offset_of!(BootInfo, framebuffer) == 72);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_incompatible_and_overflowing_handoffs() {
        let regions = [MemoryRegion::usable(0x1000, 0x10000)];
        let mut info = BootInfo::new(
            &regions,
            0x1000,
            0x2000,
            KERNEL_IMAGE_BASE,
            0x1000,
            regions.as_ptr() as u64,
        );
        assert_eq!(info.validate(), Ok(()));
        info.version = 1;
        assert!(info.validate().is_err());
        info.version = BOOT_INFO_VERSION;
        info.memory_regions_virtual += 1;
        assert!(info.validate().is_err());
        info.memory_regions_virtual = u64::MAX - 7;
        assert!(info.validate().is_err());
        info.memory_regions_virtual = regions.as_ptr() as u64;
        info.boot_info_physical = u64::MAX - 7;
        assert!(info.validate().is_err());
        info.boot_info_physical = 0x1000;
        info.kernel_virtual_start = u64::MAX - 7;
        assert!(info.validate().is_err());
        info.kernel_virtual_start = KERNEL_IMAGE_BASE;
        info.memory_region_count = 4097;
        assert!(info.validate().is_err());
    }

    #[test]
    fn device_alias_must_not_overlap_protected_physical_pages() {
        let mut fb = Framebuffer {
            base: 0x2fff,
            size: 1,
            virtual_base: MMIO_BASE,
            ..Framebuffer::NONE
        };
        assert!(fb.overlaps(0x2000, 0x2fff));
        assert!(!fb.overlaps(0x3000, 0x4000));
        fb.base = u64::MAX;
        assert!(fb.overlaps(0x1000, 0x2000));
        let regions = [MemoryRegion::usable(0x1000, 0x10000)];
        let mut info = BootInfo::new(
            &regions,
            0x2000,
            0x3000,
            KERNEL_IMAGE_BASE,
            0x1000,
            regions.as_ptr() as u64,
        );
        fb.base = info.kernel_start;
        info.framebuffer = fb;
        assert!(info.validate().is_err());
        info.framebuffer.base = info.boot_info_physical;
        assert!(info.validate().is_err());
        info.framebuffer.base = info.memory_regions;
        assert!(info.validate().is_err());
    }
}
