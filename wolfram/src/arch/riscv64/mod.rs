//! RISC-V 64-bit architecture layer.
//! The kernel core never imports from here directly.
//! Everything goes through the HAL boundary.

pub mod context;
pub mod device_tree;
pub mod paging;
pub mod serial;
pub mod trap;

#[allow(unused_imports)]
pub use context::{CpuContext, switch_context};
pub use paging::Sv39AddressSpace as ArchAddressSpace;

pub fn init() {
    trap::init();
}
