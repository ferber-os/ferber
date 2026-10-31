pub const BOOT_MAGIC: u64 = 0x574f_4c46_5241_4d21;
#[path = "../../../shared/boot_payload.rs"]
pub mod payload;
pub use payload::BootPayload;
#[path = "../../../shared/acpi.rs"]
pub mod acpi;
pub use acpi::AcpiSnapshot;
pub const BOOT_VERSION: u32 = 4;
pub const BOOT_DATA_BASE: u64 = 0xffff_fffe_0000_0000;
pub const BOOT_INFO_VA: u64 = BOOT_DATA_BASE;
pub const BOOT_REGIONS_VA: u64 = BOOT_DATA_BASE + 0x1_0000;
pub const MMIO_BASE: u64 = 0xffff_fffe_8000_0000;
pub const KERNEL_IMAGE_BASE: u64 = 0xffff_ffff_8000_0000;
pub const REGION_USABLE: u32 = 1;
pub const REGION_RESERVED: u32 = 2;
pub const REGION_RAM_RESERVED: u32 = 3;
/// Preserve RAM provenance even when firmware memory is not allocatable.
pub const fn region_kind(firmware_kind: u32) -> u32 {
    match firmware_kind {
        crate::uefi::EFI_CONVENTIONAL_MEMORY => REGION_USABLE,
        1..=6 | 8..=10 | 13..=15 => REGION_RAM_RESERVED,
        _ => REGION_RESERVED,
    }
}

pub const PIXEL_NONE: u32 = 0;
pub const PIXEL_RGB: u32 = 1;
pub const PIXEL_BGR: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MemoryRegion {
    pub start: u64,
    pub end: u64,
    pub kind: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Framebuffer {
    pub base: u64,
    pub virtual_base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: u32,
}

impl Framebuffer {
    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        self.size != 0
            && self
                .base
                .checked_add(self.size)
                .and_then(|end| end.checked_add(4095))
                .is_none_or(|fb_end| (self.base & !4095) < end && start < (fb_end & !4095))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
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
    pub boot_payload: BootPayload,
    pub acpi: AcpiSnapshot,
}

const _: () = assert!(core::mem::size_of::<MemoryRegion>() == 24);
const _: () = assert!(core::mem::size_of::<Framebuffer>() == 40);
const _: () = assert!(core::mem::size_of::<BootInfo>() == 168);
const _: () = assert!(core::mem::offset_of!(BootInfo, kernel_virtual_start) == 48);
const _: () = assert!(core::mem::offset_of!(BootInfo, framebuffer) == 72);
