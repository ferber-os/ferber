//! Validated platform descriptions. Device ownership and register access are separate.
#![allow(dead_code, unused_imports)]
pub use crate::boot_info::acpi;
pub mod mmio;
pub mod pci;
#[path = "../../../../../shared/acpi_tables.rs"]
pub mod tables;
pub use mmio::{MappedRegion, MmioMapper};
/// Retained owned firmware tables, borrowed through the boot handoff lifetime.
pub struct Discovery<'a> {
    pub snapshot: acpi::Snapshot<'a>,
}
impl<'a> Discovery<'a> {
    /// # Safety
    /// `info`'s snapshot must remain mapped read-only for the returned lifetime.
    pub unsafe fn from_boot_info(
        info: &'a crate::boot_info::BootInfo,
    ) -> Result<Self, acpi::Error> {
        info.validate().map_err(|_| acpi::Error)?;
        if info.acpi.bounds()?.is_none() {
            return Err(acpi::Error);
        }
        let bytes = core::slice::from_raw_parts(
            info.acpi.virtual_start as *const u8,
            info.acpi.length as usize,
        );
        Ok(Self {
            snapshot: acpi::Snapshot::parse(bytes)?,
        })
    }
    pub fn mcfg(&self) -> Result<tables::Mcfg<'a>, acpi::Error> {
        tables::Mcfg::parse(
            self.snapshot
                .table(b"MCFG\0\0\0\0")
                .ok_or(acpi::Error)?
                .bytes,
        )
    }
    pub fn madt(&self) -> Result<tables::Madt<'a>, acpi::Error> {
        tables::Madt::parse(
            self.snapshot
                .table(b"APIC\0\0\0\0")
                .ok_or(acpi::Error)?
                .bytes,
        )
    }
    pub fn hpet(&self) -> Result<tables::Hpet, acpi::Error> {
        tables::Hpet::parse(
            self.snapshot
                .table(b"HPET\0\0\0\0")
                .ok_or(acpi::Error)?
                .bytes,
        )
    }
    pub fn ivrs(&self) -> Result<tables::Ivrs<'a>, acpi::Error> {
        tables::Ivrs::parse(
            self.snapshot
                .table(b"IVRS\0\0\0\0")
                .ok_or(acpi::Error)?
                .bytes,
        )
    }
}

pub mod live_pci;

pub mod hpet;
pub mod iommu;
pub mod xhci;
