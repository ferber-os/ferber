//! RISC-V 64-bit architecture layer.
//! The kernel core never imports from here directly.
//! Everything goes through the HAL boundary.

pub mod context;
pub mod device_tree;
pub mod irq;
pub mod paging;
pub mod serial;
pub mod trap;
mod user;

pub use user::{
    cooperative_probe, drop_to_user, isolation_address, isolation_fault_offset, isolation_probe,
    user_probe,
};

#[allow(unused_imports)]
pub use context::{switch_context, CpuContext};
pub use paging::Sv39AddressSpace as ArchAddressSpace;

pub fn init() {
    trap::init();
}

pub use paging::{
    activate_kernel_address_space, bootstrap_aliases, finalize_kernel_space,
    physical_kernel_bounds, runtime_physical_base,
};

/// Check supervisor trap state before switching a kernel continuation.
///
/// # Safety
/// Call only in supervisor mode with interrupts disabled and a retained stack.
pub unsafe fn bind_user_kernel_stack(kernel_sp: usize) {
    assert_eq!(kernel_sp & 15, 0);
    let scratch: usize;
    core::arch::asm!("csrr {}, sscratch", out(reg) scratch, options(nostack));
    assert_eq!(scratch, 0, "scheduler entered with a user trap stack armed");
}
