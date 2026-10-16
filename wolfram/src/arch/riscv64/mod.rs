//! RISC-V 64-bit architecture layer.
//! The kernel core never imports from here directly.
//! Everything goes through the HAL boundary.

pub mod context;
pub mod device_tree;
pub mod paging;
pub mod serial;
pub mod trap;
mod user;

pub use user::{
    drop_to_user, isolation_address, isolation_fault_offset, isolation_probe, user_probe,
};

#[allow(unused_imports)]
pub use context::{switch_context, CpuContext};
pub use paging::Sv39AddressSpace as ArchAddressSpace;

pub fn init() {
    trap::init();
}

pub use paging::{
    bootstrap_aliases, finalize_kernel_space, physical_kernel_bounds, runtime_physical_base,
};
