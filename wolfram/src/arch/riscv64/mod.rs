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

/// Install the bootstrap supervisor mappings in a new user address space.
///
/// # Safety
/// The RAM interval must be page-aligned, resident RAM containing the running
/// kernel and all physical aliases. Keep the returned VMO alive while active.
pub unsafe fn prepare_user_space(
    space: &mut ArchAddressSpace,
    ram_start: usize,
    ram_end: usize,
) -> crate::kernel::memory::vmo::Vmo {
    use crate::kernel::memory::paging::{AddressSpace, MapFlags};
    use crate::kernel::memory::vmo::Vmo;
    extern "C" {
        static __text_start: u8;
        static __text_end: u8;
        static __rodata_start: u8;
        static __rodata_end: u8;
    }
    assert!(ram_start < ram_end && ram_start.is_multiple_of(4096) && ram_end.is_multiple_of(4096));
    // shortcut: retain identity RAM aliases until the permanent kernel layout replaces boot mappings.
    let backing = Vmo::create_physical(ram_start, ram_end - ram_start).expect("RAM backing VMO");
    space
        .map_vmo(ram_start, backing.frames(), MapFlags::kernel_rw())
        .expect("supervisor RAM mappings");
    let text = (&__text_start as *const u8 as usize)..(&__text_end as *const u8 as usize);
    let rodata = (&__rodata_start as *const u8 as usize)..(&__rodata_end as *const u8 as usize);
    space
        .protect(text.start, text.len() / 4096, MapFlags::kernel_rx())
        .expect("kernel text protection");
    space
        .protect(rodata.start, rodata.len() / 4096, MapFlags::kernel_ro())
        .expect("kernel rodata protection");
    backing
}
