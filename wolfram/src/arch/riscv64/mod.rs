//! RISC-V 64-bit architecture layer.
//! The kernel core never imports from here directly.
//! Everything goes through the HAL boundary.

pub mod device_tree;
pub mod paging;
pub mod serial;
pub mod trap;

pub use paging::Sv39AddressSpace as ArchAddressSpace;

pub fn init() {
    trap::init();
}
