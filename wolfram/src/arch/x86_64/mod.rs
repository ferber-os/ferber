//! x86-64 early architecture support. Firmware services end before entry.

pub mod console;
pub mod context;
pub mod paging;
pub mod syscall;
pub mod trap;

#[allow(unused_imports)]
pub use context::{switch_context, CpuContext};
pub use paging::Pml4AddressSpace as ArchAddressSpace;

pub use syscall::{
    drop_to_user, isolation_address, isolation_fault_offset, isolation_probe, user_probe,
};

use crate::boot_info::BootInfo;

pub fn init(boot_info: &BootInfo) {
    console::init(&boot_info.framebuffer);
    paging::init();
    trap::init();
    syscall::init();
}
