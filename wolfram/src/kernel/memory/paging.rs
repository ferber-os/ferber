//! Architecture-agnostic paging HAL.
//!
//! The kernel core never touches hardware page tables directly.
//! Each arch module implements this trait; the memory layer calls only these methods.
//!
//! Current implementors: x86_64 (4-level PML4), riscv64 (Sv39).

#![allow(dead_code)]

use crate::kernel::memory::vmo::VmoFrames;

/// Page-level rights for a mapping. Enforced at the architecture layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapFlags {
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
    pub user: bool,
}

impl MapFlags {
    pub const fn kernel_ro() -> Self {
        Self { readable: true, writable: false, executable: false, user: false }
    }
    pub const fn kernel_rw() -> Self {
        Self { readable: true, writable: true, executable: false, user: false }
    }
    pub const fn kernel_rx() -> Self {
        Self { readable: true, writable: false, executable: true, user: false }
    }
    pub const fn user_ro() -> Self {
        Self { readable: true, writable: false, executable: false, user: true }
    }
    pub const fn user_rw() -> Self {
        Self { readable: true, writable: true, executable: false, user: true }
    }
    pub const fn user_rx() -> Self {
        Self { readable: true, writable: false, executable: true, user: true }
    }
}

/// Errors returned by paging operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageError {
    /// Physical frame allocation failed (OOM).
    OutOfMemory,
    /// Virtual address range is not page-aligned.
    Unaligned,
    /// Virtual address range is out of bounds for this address space.
    OutOfRange,
    /// The mapping already exists (no implicit overwrite).
    AlreadyMapped,
}

/// The paging HAL. Implemented once per architecture.
///
/// # Safety
/// Implementations manipulate hardware page tables. All methods are unsafe.
/// The caller is responsible for ensuring addresses are valid and consistent.
pub trait AddressSpace {
    /// Map a single physical frame at `virt_addr` with `flags`.
    ///
    /// - `virt_addr` and `phys_addr` must both be 4 KiB aligned.
    /// - Returns `Err(AlreadyMapped)` if the entry is already present.
    unsafe fn map_page(
        &mut self,
        virt_addr: usize,
        phys_addr: usize,
        flags: MapFlags,
    ) -> Result<(), PageError>;

    /// Unmap the page at `virt_addr`. Does NOT free the underlying physical frame.
    ///
    /// - `virt_addr` must be 4 KiB aligned.
    /// - Returns `Ok(phys_addr)` of the now-unmapped frame.
    unsafe fn unmap_page(&mut self, virt_addr: usize) -> Result<usize, PageError>;

    /// Map contiguous frames from `frames` starting at `virt_addr`.
    ///
    /// On partial failure, already-mapped pages are unmapped before returning.
    unsafe fn map_vmo(
        &mut self,
        virt_addr: usize,
        frames: &VmoFrames,
        flags: MapFlags,
    ) -> Result<(), PageError> {
        const PAGE_SIZE: usize = 4096;
        let mut mapped = 0;
        for (i, &phys) in frames.as_slice().iter().enumerate() {
            match self.map_page(virt_addr + i * PAGE_SIZE, phys, flags) {
                Ok(()) => mapped += 1,
                Err(e) => {
                    for j in 0..mapped {
                        let _ = self.unmap_page(virt_addr + j * PAGE_SIZE);
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Unmap all pages that were mapped by `map_vmo`. Does NOT free frames.
    unsafe fn unmap_vmo(&mut self, virt_addr: usize, frame_count: usize) -> Result<(), PageError> {
        const PAGE_SIZE: usize = 4096;
        for i in 0..frame_count {
            self.unmap_page(virt_addr + i * PAGE_SIZE)?;
        }
        Ok(())
    }

    /// Flush the TLB for a single page. Must be called after any map/unmap.
    unsafe fn flush_tlb_page(&self, virt_addr: usize);

    /// Flush the entire TLB (expensive — use only when switching address spaces).
    unsafe fn flush_tlb_all(&self);
}
