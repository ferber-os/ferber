//! Validated platform descriptions. Device ownership and register access are separate.
pub use crate::boot_info::acpi;
pub mod pci;
#[path = "../../../../../shared/acpi_tables.rs"]
pub mod tables;
