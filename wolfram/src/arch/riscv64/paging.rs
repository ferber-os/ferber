//! RISC-V 64 Sv39 multi-level paging implementation.
//!
//! Adheres strictly to the RISC-V Privileged Architecture Specification
//! (Volume II: Privileged Architecture, Chapter 4: Supervisor-Level Memory Management for Sv39).
//!
//! Sv39 translates 39-bit canonical virtual addresses to 56-bit physical addresses
//! across 3 page table levels:
//! - Level 2 (root): VPN[2] (bits 38..30)
//! - Level 1:        VPN[1] (bits 29..21)
//! - Level 0 (leaf): VPN[0] (bits 20..12)
//! - Offset:         bits 11..0

#![allow(dead_code)]

use crate::kernel::memory::bitmap;
use crate::kernel::memory::paging::{AddressSpace, MapFlags, PageError};

/// 4 KiB page size.
pub const PAGE_SIZE: usize = 4096;

/// Number of 64-bit page table entries per 4 KiB table page.
pub const ENTRIES_PER_TABLE: usize = 512;

/// PTE Valid bit. When 1, entry is valid.
pub const PTE_V: u64 = 1 << 0;
/// PTE Readable bit.
pub const PTE_R: u64 = 1 << 1;
/// PTE Writable bit.
pub const PTE_W: u64 = 1 << 2;
/// PTE Executable bit.
pub const PTE_X: u64 = 1 << 3;
/// PTE User-accessible bit.
pub const PTE_U: u64 = 1 << 4;
/// PTE Global mapping bit.
pub const PTE_G: u64 = 1 << 5;
/// PTE Accessed bit.
pub const PTE_A: u64 = 1 << 6;
/// PTE Dirty bit.
pub const PTE_D: u64 = 1 << 7;

/// Mask for 44-bit Physical Page Number (PPN) in PTE (bits 53..10).
pub const PPN_MASK: u64 = 0x0fff_ffff_ffff;

/// Returns true if the virtual address is canonical in Sv39.
///
/// In Sv39, a 64-bit virtual address is canonical if bits 63..39 match bit 38
/// (i.e. bit 38 is sign-extended through bits 63..39).
#[inline]
pub fn is_canonical(va: usize) -> bool {
    let sign_extended = ((va as isize) << (64 - 39)) >> (64 - 39);
    sign_extended as usize == va
}

/// Extract Level 2 Virtual Page Number index (bits 38..30).
#[inline]
pub fn vpn2(va: usize) -> usize {
    (va >> 30) & 0x1ff
}

/// Extract Level 1 Virtual Page Number index (bits 29..21).
#[inline]
pub fn vpn1(va: usize) -> usize {
    (va >> 21) & 0x1ff
}

/// Extract Level 0 Virtual Page Number index (bits 20..12).
#[inline]
pub fn vpn0(va: usize) -> usize {
    (va >> 12) & 0x1ff
}

/// Extract page offset (bits 11..0).
#[inline]
pub fn page_offset(va: usize) -> usize {
    va & 0xfff
}

/// Convert a PTE to its target 4 KiB-aligned physical address.
#[inline]
fn pte_to_paddr(pte: u64) -> usize {
    (((pte >> 10) & PPN_MASK) << 12) as usize
}

/// Construct a non-leaf PTE pointing to a next-level page table.
#[inline]
fn make_table_pte(paddr: usize) -> u64 {
    ((paddr as u64 >> 12) << 10) | PTE_V
}

/// Construct a leaf PTE mapping a 4 KiB physical page with the given flags.
///
/// Sets V, A, and D bits. In Sv39, writable implies readable (W requires R).
#[inline]
fn make_leaf_pte(paddr: usize, flags: MapFlags) -> u64 {
    let mut pte = ((paddr as u64 >> 12) << 10) | PTE_V | PTE_A | PTE_D;
    if flags.readable || flags.writable {
        pte |= PTE_R;
    }
    if flags.writable {
        pte |= PTE_W;
    }
    if flags.executable {
        pte |= PTE_X;
    }
    if flags.user {
        pte |= PTE_U;
    }
    pte
}

/// Allocate a 4 KiB frame for a page table and zero its contents.
fn allocate_table() -> Result<usize, PageError> {
    let paddr = bitmap::allocate_frame().ok_or(PageError::OutOfMemory)?;
    unsafe {
        core::ptr::write_bytes(paddr as *mut u8, 0, PAGE_SIZE);
    }
    Ok(paddr)
}

/// Sv39 page table address space.
#[derive(Debug)]
pub struct Sv39AddressSpace {
    root_paddr: usize,
}

impl Sv39AddressSpace {
    /// Creates a new Sv39 address space by allocating a root Level 2 page table.
    pub fn new() -> Result<Self, PageError> {
        let root_paddr = allocate_table()?;
        Ok(Self { root_paddr })
    }

    /// Returns the physical address of the root Level 2 table.
    pub fn root_paddr(&self) -> usize {
        self.root_paddr
    }
}

impl AddressSpace for Sv39AddressSpace {
    unsafe fn map_page(
        &mut self,
        virt_addr: usize,
        phys_addr: usize,
        flags: MapFlags,
    ) -> Result<(), PageError> {
        if !virt_addr.is_multiple_of(PAGE_SIZE) || !phys_addr.is_multiple_of(PAGE_SIZE) {
            return Err(PageError::Unaligned);
        }
        if !is_canonical(virt_addr) {
            return Err(PageError::OutOfRange);
        }

        // Level 2 walk
        let l2_table = self.root_paddr as *mut u64;
        let idx2 = vpn2(virt_addr);
        let pte2 = unsafe { l2_table.add(idx2).read() };
        let l1_table = if (pte2 & PTE_V) == 0 {
            let l1_paddr = allocate_table()?;
            unsafe {
                l2_table.add(idx2).write(make_table_pte(l1_paddr));
            }
            l1_paddr as *mut u64
        } else if (pte2 & (PTE_R | PTE_W | PTE_X)) != 0 {
            // Already mapped as a 1 GiB superpage leaf
            return Err(PageError::AlreadyMapped);
        } else {
            pte_to_paddr(pte2) as *mut u64
        };

        // Level 1 walk
        let idx1 = vpn1(virt_addr);
        let pte1 = unsafe { l1_table.add(idx1).read() };
        let l0_table = if (pte1 & PTE_V) == 0 {
            let l0_paddr = allocate_table()?;
            unsafe {
                l1_table.add(idx1).write(make_table_pte(l0_paddr));
            }
            l0_paddr as *mut u64
        } else if (pte1 & (PTE_R | PTE_W | PTE_X)) != 0 {
            // Already mapped as a 2 MiB superpage leaf
            return Err(PageError::AlreadyMapped);
        } else {
            pte_to_paddr(pte1) as *mut u64
        };

        // Level 0 walk & leaf insertion
        let idx0 = vpn0(virt_addr);
        let pte0 = unsafe { l0_table.add(idx0).read() };
        if (pte0 & PTE_V) != 0 {
            return Err(PageError::AlreadyMapped);
        }

        let leaf_pte = make_leaf_pte(phys_addr, flags);
        unsafe {
            l0_table.add(idx0).write(leaf_pte);
        }

        self.flush_tlb_page(virt_addr);
        Ok(())
    }

    unsafe fn unmap_page(&mut self, virt_addr: usize) -> Result<usize, PageError> {
        if !virt_addr.is_multiple_of(PAGE_SIZE) {
            return Err(PageError::Unaligned);
        }
        if !is_canonical(virt_addr) {
            return Err(PageError::OutOfRange);
        }

        // Level 2 walk
        let l2_table = self.root_paddr as *mut u64;
        let idx2 = vpn2(virt_addr);
        let pte2 = unsafe { l2_table.add(idx2).read() };
        if (pte2 & PTE_V) == 0 || (pte2 & (PTE_R | PTE_W | PTE_X)) != 0 {
            return Err(PageError::NotMapped);
        }
        let l1_table = pte_to_paddr(pte2) as *mut u64;

        // Level 1 walk
        let idx1 = vpn1(virt_addr);
        let pte1 = unsafe { l1_table.add(idx1).read() };
        if (pte1 & PTE_V) == 0 || (pte1 & (PTE_R | PTE_W | PTE_X)) != 0 {
            return Err(PageError::NotMapped);
        }
        let l0_table = pte_to_paddr(pte1) as *mut u64;

        // Level 0 walk
        let idx0 = vpn0(virt_addr);
        let pte0 = unsafe { l0_table.add(idx0).read() };
        if (pte0 & PTE_V) == 0 {
            return Err(PageError::NotMapped);
        }

        let phys_addr = pte_to_paddr(pte0);
        unsafe {
            l0_table.add(idx0).write(0);
        }

        self.flush_tlb_page(virt_addr);
        Ok(phys_addr)
    }

    unsafe fn protect_page(&mut self, virt_addr: usize, flags: MapFlags) -> Result<(), PageError> {
        if !virt_addr.is_multiple_of(PAGE_SIZE) {
            return Err(PageError::Unaligned);
        }
        if !is_canonical(virt_addr) {
            return Err(PageError::OutOfRange);
        }

        // Level 2 walk
        let l2_table = self.root_paddr as *mut u64;
        let idx2 = vpn2(virt_addr);
        let pte2 = unsafe { l2_table.add(idx2).read() };
        if (pte2 & PTE_V) == 0 || (pte2 & (PTE_R | PTE_W | PTE_X)) != 0 {
            return Err(PageError::NotMapped);
        }
        let l1_table = pte_to_paddr(pte2) as *mut u64;

        // Level 1 walk
        let idx1 = vpn1(virt_addr);
        let pte1 = unsafe { l1_table.add(idx1).read() };
        if (pte1 & PTE_V) == 0 || (pte1 & (PTE_R | PTE_W | PTE_X)) != 0 {
            return Err(PageError::NotMapped);
        }
        let l0_table = pte_to_paddr(pte1) as *mut u64;

        // Level 0 walk
        let idx0 = vpn0(virt_addr);
        let pte0 = unsafe { l0_table.add(idx0).read() };
        if (pte0 & PTE_V) == 0 {
            return Err(PageError::NotMapped);
        }

        let phys_addr = pte_to_paddr(pte0);
        let new_pte = make_leaf_pte(phys_addr, flags);
        unsafe {
            l0_table.add(idx0).write(new_pte);
        }

        self.flush_tlb_page(virt_addr);
        Ok(())
    }

    fn query(&self, virt_addr: usize) -> Option<(usize, MapFlags)> {
        if !is_canonical(virt_addr) {
            return None;
        }

        // Level 2 walk
        let l2_table = self.root_paddr as *const u64;
        let idx2 = vpn2(virt_addr);
        let pte2 = unsafe { l2_table.add(idx2).read() };
        if (pte2 & PTE_V) == 0 {
            return None;
        }
        if (pte2 & (PTE_R | PTE_W | PTE_X)) != 0 {
            // 1 GiB superpage leaf: PPN[2] with VA bits 29..0
            let paddr = (pte_to_paddr(pte2) & !0x3fff_ffff) | (virt_addr & 0x3fff_ffff);
            let flags = MapFlags {
                readable: (pte2 & PTE_R) != 0,
                writable: (pte2 & PTE_W) != 0,
                executable: (pte2 & PTE_X) != 0,
                user: (pte2 & PTE_U) != 0,
            };
            return Some((paddr, flags));
        }
        let l1_table = pte_to_paddr(pte2) as *const u64;

        // Level 1 walk
        let idx1 = vpn1(virt_addr);
        let pte1 = unsafe { l1_table.add(idx1).read() };
        if (pte1 & PTE_V) == 0 {
            return None;
        }
        if (pte1 & (PTE_R | PTE_W | PTE_X)) != 0 {
            // 2 MiB superpage leaf: PPN[2..1] with VA bits 20..0
            let paddr = (pte_to_paddr(pte1) & !0x1f_ffff) | (virt_addr & 0x1f_ffff);
            let flags = MapFlags {
                readable: (pte1 & PTE_R) != 0,
                writable: (pte1 & PTE_W) != 0,
                executable: (pte1 & PTE_X) != 0,
                user: (pte1 & PTE_U) != 0,
            };
            return Some((paddr, flags));
        }
        let l0_table = pte_to_paddr(pte1) as *const u64;

        // Level 0 walk
        let idx0 = vpn0(virt_addr);
        let pte0 = unsafe { l0_table.add(idx0).read() };
        if (pte0 & PTE_V) == 0 {
            return None;
        }

        // 4 KiB leaf page: PPN[2..0] with VA bits 11..0
        let paddr = pte_to_paddr(pte0) | (virt_addr & 0xfff);
        let flags = MapFlags {
            readable: (pte0 & PTE_R) != 0,
            writable: (pte0 & PTE_W) != 0,
            executable: (pte0 & PTE_X) != 0,
            user: (pte0 & PTE_U) != 0,
        };
        Some((paddr, flags))
    }

    unsafe fn activate(&self) {
        let satp = (8usize << 60) | (self.root_paddr >> 12);
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma zero, zero",
            satp = in(reg) satp,
            options(nostack)
        );
    }

    unsafe fn flush_tlb_page(&self, virt_addr: usize) {
        core::arch::asm!(
            "sfence.vma {va}, zero",
            va = in(reg) virt_addr,
            options(nostack)
        );
    }

    unsafe fn flush_tlb_all(&self) {
        core::arch::asm!(
            "sfence.vma zero, zero",
            options(nostack)
        );
    }
}

impl Drop for Sv39AddressSpace {
    fn drop(&mut self) {
        if self.root_paddr == 0 {
            return;
        }

        let l2_table = self.root_paddr as *mut u64;
        for i2 in 0..ENTRIES_PER_TABLE {
            let pte2 = unsafe { l2_table.add(i2).read() };
            if (pte2 & PTE_V) != 0 && (pte2 & (PTE_R | PTE_W | PTE_X)) == 0 {
                let l1_paddr = pte_to_paddr(pte2);
                let l1_table = l1_paddr as *mut u64;
                for i1 in 0..ENTRIES_PER_TABLE {
                    let pte1 = unsafe { l1_table.add(i1).read() };
                    if (pte1 & PTE_V) != 0 && (pte1 & (PTE_R | PTE_W | PTE_X)) == 0 {
                        let l0_paddr = pte_to_paddr(pte1);
                        let _ = bitmap::free_frame(l0_paddr);
                    }
                }
                let _ = bitmap::free_frame(l1_paddr);
            }
        }
        let _ = bitmap::free_frame(self.root_paddr);
        self.root_paddr = 0;
    }
}
