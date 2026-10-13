//! x86-64 early architecture support. Firmware services end before entry.

pub mod console;
pub mod context;
pub mod trap;
pub mod syscall;
pub mod paging;

#[allow(unused_imports)]
pub use context::{CpuContext, switch_context};
pub use paging::Pml4AddressSpace as ArchAddressSpace;

use crate::boot_info::BootInfo;

pub fn init(boot_info: &BootInfo) {
    console::init(&boot_info.framebuffer);
    trap::init();
}
