//! x86-64 4-level paging (PML4) implementation.
//!
//! Adheres strictly to the Intel 64 and AMD64 Architecture Manuals
//! (4-level PML4 paging in long mode).
//!
//! PML4 translates 48-bit canonical virtual addresses across 4 page table levels:
//! - Level 4 (PML4): VPN[3] (bits 47..39) -> entries 0..511
//! - Level 3 (PDPT): VPN[2] (bits 38..30) -> entries 0..511
//! - Level 2 (PD):   VPN[1] (bits 29..21) -> entries 0..511
//! - Level 1 (PT):   VPN[0] (bits 20..12) -> entries 0..511
//! - Offset:         bits 11..0
//!
//! Canonical address spaces:
//! - User space:   0x0000_0000_0000_0000..=0x0000_7FFF_FFFF_FFFF (PML4 entries 0..255)
//! - Kernel space: 0xFFFF_8000_0000_0000..=0xFFFF_FFFF_FFFF_FFFF (PML4 entries 256..511)
//!
//! Non-canonical gap: addresses outside these ranges return Err(PageError::OutOfRange).

#![allow(dead_code)]

use crate::kernel::memory::bitmap;
use crate::kernel::memory::paging::{AddressSpace, MapFlags, PageError};

/// 4 KiB page size.
pub const PAGE_SIZE: usize = 4096;

/// Number of 64-bit page table entries per 4 KiB table page.
pub const ENTRIES_PER_TABLE: usize = 512;

/// PTE Present bit (bit 0). When set, the page or next-level page table is present in physical memory.
pub const PTE_PRESENT: u64 = 1 << 0;
/// PTE Writable bit (bit 1). When set, writes are allowed to the 4 KiB page or through this directory entry.
pub const PTE_WRITABLE: u64 = 1 << 1;
/// PTE User-accessible bit (bit 2). When set, user-mode (ring 3) access is allowed.
pub const PTE_USER: u64 = 1 << 2;
/// PTE Page-level Write-Through bit (bit 3).
pub const PTE_PWT: u64 = 1 << 3;
/// PTE Page-level Cache Disable bit (bit 4).
pub const PTE_PCD: u64 = 1 << 4;
/// PTE Accessed bit (bit 5). Set by hardware when the page or table is accessed.
pub const PTE_ACCESSED: u64 = 1 << 5;
/// PTE Dirty bit (bit 6). Set by hardware when the page is written to.
pub const PTE_DIRTY: u64 = 1 << 6;
/// PTE Page Size / Huge page bit (bit 7). When set in PDPT (1 GiB) or PD (2 MiB), entry is a leaf superpage.
pub const PTE_HUGE: u64 = 1 << 7;
/// PTE Global bit (bit 8). When set, TLB entry is not flushed on CR3 reload.
pub const PTE_GLOBAL: u64 = 1 << 8;
/// PTE No-Execute bit (bit 63). When set, instruction fetches are disabled.
pub const PTE_NO_EXECUTE: u64 = 1 << 63;

/// Mask for physical address in page table entries (bits 51..12).
pub const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

/// Returns true if the virtual address is canonical in 48-bit x86-64 long mode.
///
/// In 48-bit long mode, a virtual address is canonical if bits 63..47 match bit 47
/// (i.e. bit 47 is sign-extended through bits 63..48).
/// - User space:   `0x0000_0000_0000_0000..=0x0000_7FFF_FFFF_FFFF` (entries 0..255)
/// - Kernel space: `0xFFFF_8000_0000_0000..=0xFFFF_FFFF_FFFF_FFFF` (entries 256..511)
#[inline]
pub fn is_canonical(va: usize) -> bool {
    let sign_extended = ((va as isize) << (64 - 48)) >> (64 - 48);
    sign_extended as usize == va
}

/// Extract Level 4 PML4 index (bits 47..39).
#[inline]
pub fn pml4_index(va: usize) -> usize {
    (va >> 39) & 0x1ff
}

/// Extract Level 3 PDPT index (bits 38..30).
#[inline]
pub fn pdpt_index(va: usize) -> usize {
    (va >> 30) & 0x1ff
}

/// Extract Level 2 PD index (bits 29..21).
#[inline]
pub fn pd_index(va: usize) -> usize {
    (va >> 21) & 0x1ff
}

/// Extract Level 1 PT index (bits 20..12).
#[inline]
pub fn pt_index(va: usize) -> usize {
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
    (pte & ADDRESS_MASK) as usize
}

/// Construct a non-leaf intermediate PTE pointing to a child page table.
#[inline]
fn make_table_entry(paddr: usize) -> u64 {
    (paddr as u64 & ADDRESS_MASK) | PTE_PRESENT | PTE_WRITABLE | PTE_USER
}

/// Construct a leaf PTE mapping a 4 KiB physical page with the given flags.
///
/// Sets Present, Accessed, and Dirty bits.
/// Enforces Writable, User, and No-Execute permissions according to `flags`.
#[inline]
fn make_leaf_entry(paddr: usize, flags: MapFlags) -> u64 {
    let mut pte = (paddr as u64 & ADDRESS_MASK) | PTE_PRESENT | PTE_ACCESSED | PTE_DIRTY;
    if flags.writable {
        pte |= PTE_WRITABLE;
    }
    if flags.user {
        pte |= PTE_USER;
    }
    if !flags.executable {
        pte |= PTE_NO_EXECUTE;
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

/// x86-64 PML4 page table address space.
#[derive(Debug)]
pub struct Pml4AddressSpace {
    root_paddr: usize,
}

static BOOT_ROOT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Capture the loader-owned tables before a process root can become active.
pub fn init() {
    let root: usize;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) root, options(nomem, nostack));
    }
    BOOT_ROOT
        .compare_exchange(
            0,
            root & ADDRESS_MASK as usize,
            core::sync::atomic::Ordering::SeqCst,
            core::sync::atomic::Ordering::SeqCst,
        )
        .expect("boot CR3 initialized twice");
}

unsafe fn free_table(table: usize, level: usize, count: usize) {
    for i in 0..count {
        let entry = (table as *const u64).add(i).read();
        if level > 1 && entry & PTE_PRESENT != 0 && entry & PTE_HUGE == 0 {
            free_table(pte_to_paddr(entry), level - 1, 512);
        }
    }
    bitmap::free_frame(table).expect("owned page table frame");
}

unsafe fn clone_table(source: usize, level: usize) -> Result<usize, PageError> {
    let target = allocate_table()?;
    for i in 0..512 {
        let entry = (source as *const u64).add(i).read();
        let cloned = if entry & PTE_PRESENT != 0 && level > 1 && entry & PTE_HUGE == 0 {
            match clone_table(pte_to_paddr(entry), level - 1) {
                Ok(child) => (entry & !ADDRESS_MASK) | child as u64,
                Err(error) => {
                    free_table(target, level, 512);
                    return Err(error);
                }
            }
        } else {
            entry
        };
        (target as *mut u64).add(i).write(cloned);
    }
    Ok(target)
}

// Preserve sibling restrictions before opening an ancestor for a user mapping.
unsafe fn open_table(slot: *mut u64, flags: MapFlags) {
    let mut entry = slot.read();
    let relax_user = flags.user && entry & PTE_USER == 0;
    let relax_write = flags.writable && entry & PTE_WRITABLE == 0;
    let relax_execute = flags.executable && entry & PTE_NO_EXECUTE != 0;
    if relax_user || relax_write || relax_execute {
        let child = pte_to_paddr(entry) as *mut u64;
        for i in 0..512 {
            let mut value = child.add(i).read();
            if value & PTE_PRESENT == 0 {
                continue;
            }
            if relax_user {
                value &= !PTE_USER;
            }
            if relax_write {
                value &= !PTE_WRITABLE;
            }
            if relax_execute {
                value |= PTE_NO_EXECUTE;
            }
            child.add(i).write(value);
        }
        if relax_user {
            entry |= PTE_USER;
        }
        if relax_write {
            entry |= PTE_WRITABLE;
        }
        if relax_execute {
            entry &= !PTE_NO_EXECUTE;
        }
        slot.write(entry);
    }
}

impl Pml4AddressSpace {
    pub fn new() -> Result<Self, PageError> {
        let source = BOOT_ROOT.load(core::sync::atomic::Ordering::SeqCst);
        assert_ne!(
            source, 0,
            "boot CR3 must be captured before process creation"
        );
        let result = Self {
            root_paddr: allocate_table()?,
        };
        // shortcut: privately clone boot identity tables until the kernel has a permanent higher-half mapping.
        unsafe {
            core::ptr::copy_nonoverlapping(
                (source as *const u64).add(256),
                (result.root_paddr as *mut u64).add(256),
                256,
            );
            for i in 0..256 {
                let entry = (source as *const u64).add(i).read();
                if entry & PTE_PRESENT != 0 {
                    let child = clone_table(pte_to_paddr(entry), 3)?;
                    (result.root_paddr as *mut u64)
                        .add(i)
                        .write((entry & !ADDRESS_MASK) | child as u64);
                }
            }
        }
        Ok(result)
    }

    pub fn root_paddr(&self) -> usize {
        self.root_paddr
    }
}

impl AddressSpace for Pml4AddressSpace {
    unsafe fn map_page(
        &mut self,
        virt_addr: usize,
        phys_addr: usize,
        flags: MapFlags,
    ) -> Result<(), PageError> {
        if !virt_addr.is_multiple_of(PAGE_SIZE) || !phys_addr.is_multiple_of(PAGE_SIZE) {
            return Err(PageError::Unaligned);
        }
        if !is_canonical(virt_addr) || pml4_index(virt_addr) >= 256 {
            return Err(PageError::OutOfRange);
        }

        if !flags.readable {
            return Err(PageError::OutOfRange);
        }
        // Level 4 (PML4) walk
        let pml4 = self.root_paddr as *mut u64;
        let l4_idx = pml4_index(virt_addr);
        let pml4e = unsafe { pml4.add(l4_idx).read() };
        let pdpt = if (pml4e & PTE_PRESENT) == 0 {
            let pdpt_paddr = allocate_table()?;
            unsafe {
                pml4.add(l4_idx).write(make_table_entry(pdpt_paddr));
            }
            pdpt_paddr as *mut u64
        } else {
            unsafe {
                open_table(pml4.add(l4_idx), flags);
            }
            pte_to_paddr(pml4e) as *mut u64
        };

        // Level 3 (PDPT) walk
        let l3_idx = pdpt_index(virt_addr);
        let pdpte = unsafe { pdpt.add(l3_idx).read() };
        let pd = if (pdpte & PTE_PRESENT) == 0 {
            let pd_paddr = allocate_table()?;
            unsafe {
                pdpt.add(l3_idx).write(make_table_entry(pd_paddr));
            }
            pd_paddr as *mut u64
        } else if (pdpte & PTE_HUGE) != 0 {
            // Already mapped as a 1 GiB huge page leaf
            return Err(PageError::AlreadyMapped);
        } else {
            unsafe {
                open_table(pdpt.add(l3_idx), flags);
            }
            pte_to_paddr(pdpte) as *mut u64
        };

        // Level 2 (PD) walk
        let l2_idx = pd_index(virt_addr);
        let pde = unsafe { pd.add(l2_idx).read() };
        let pt = if (pde & PTE_PRESENT) == 0 {
            let pt_paddr = allocate_table()?;
            unsafe {
                pd.add(l2_idx).write(make_table_entry(pt_paddr));
            }
            pt_paddr as *mut u64
        } else if (pde & PTE_HUGE) != 0 {
            // Already mapped as a 2 MiB huge page leaf
            return Err(PageError::AlreadyMapped);
        } else {
            unsafe {
                open_table(pd.add(l2_idx), flags);
            }
            pte_to_paddr(pde) as *mut u64
        };

        // Level 1 (PT) walk & leaf insertion
        let l1_idx = pt_index(virt_addr);
        let pte = unsafe { pt.add(l1_idx).read() };
        if (pte & PTE_PRESENT) != 0 {
            return Err(PageError::AlreadyMapped);
        }

        let leaf_pte = make_leaf_entry(phys_addr, flags);
        unsafe {
            pt.add(l1_idx).write(leaf_pte);
        }

        self.flush_tlb_page(virt_addr);
        Ok(())
    }

    unsafe fn unmap_page(&mut self, virt_addr: usize) -> Result<usize, PageError> {
        if !virt_addr.is_multiple_of(PAGE_SIZE) {
            return Err(PageError::Unaligned);
        }
        if !is_canonical(virt_addr) || pml4_index(virt_addr) >= 256 {
            return Err(PageError::OutOfRange);
        }

        // Level 4 (PML4) walk
        let pml4 = self.root_paddr as *mut u64;
        let l4_idx = pml4_index(virt_addr);
        let pml4e = unsafe { pml4.add(l4_idx).read() };
        if (pml4e & PTE_PRESENT) == 0 {
            return Err(PageError::NotMapped);
        }
        let pdpt = pte_to_paddr(pml4e) as *mut u64;

        // Level 3 (PDPT) walk
        let l3_idx = pdpt_index(virt_addr);
        let pdpte = unsafe { pdpt.add(l3_idx).read() };
        if (pdpte & PTE_PRESENT) == 0 || (pdpte & PTE_HUGE) != 0 {
            return Err(PageError::NotMapped);
        }
        let pd = pte_to_paddr(pdpte) as *mut u64;

        // Level 2 (PD) walk
        let l2_idx = pd_index(virt_addr);
        let pde = unsafe { pd.add(l2_idx).read() };
        if (pde & PTE_PRESENT) == 0 || (pde & PTE_HUGE) != 0 {
            return Err(PageError::NotMapped);
        }
        let pt = pte_to_paddr(pde) as *mut u64;

        // Level 1 (PT) walk
        let l1_idx = pt_index(virt_addr);
        let pte = unsafe { pt.add(l1_idx).read() };
        if (pte & PTE_PRESENT) == 0 {
            return Err(PageError::NotMapped);
        }

        let phys_addr = pte_to_paddr(pte);
        unsafe {
            pt.add(l1_idx).write(0);
        }

        self.flush_tlb_page(virt_addr);
        Ok(phys_addr)
    }

    unsafe fn protect_page(&mut self, virt_addr: usize, flags: MapFlags) -> Result<(), PageError> {
        if !virt_addr.is_multiple_of(PAGE_SIZE) {
            return Err(PageError::Unaligned);
        }
        if !is_canonical(virt_addr) || pml4_index(virt_addr) >= 256 {
            return Err(PageError::OutOfRange);
        }

        if !flags.readable {
            return Err(PageError::OutOfRange);
        }
        // Level 4 (PML4) walk
        let pml4 = self.root_paddr as *mut u64;
        let l4_idx = pml4_index(virt_addr);
        let pml4e = unsafe { pml4.add(l4_idx).read() };
        if (pml4e & PTE_PRESENT) == 0 {
            return Err(PageError::NotMapped);
        }
        let pdpt = pte_to_paddr(pml4e) as *mut u64;

        // Level 3 (PDPT) walk
        let l3_idx = pdpt_index(virt_addr);
        let pdpte = unsafe { pdpt.add(l3_idx).read() };
        if (pdpte & PTE_PRESENT) == 0 || (pdpte & PTE_HUGE) != 0 {
            return Err(PageError::NotMapped);
        }
        let pd = pte_to_paddr(pdpte) as *mut u64;

        // Level 2 (PD) walk
        let l2_idx = pd_index(virt_addr);
        let pde = unsafe { pd.add(l2_idx).read() };
        if (pde & PTE_PRESENT) == 0 || (pde & PTE_HUGE) != 0 {
            return Err(PageError::NotMapped);
        }
        let pt = pte_to_paddr(pde) as *mut u64;

        // Level 1 (PT) walk
        let l1_idx = pt_index(virt_addr);
        let pte = unsafe { pt.add(l1_idx).read() };
        if (pte & PTE_PRESENT) == 0 {
            return Err(PageError::NotMapped);
        }

        unsafe {
            open_table(pml4.add(l4_idx), flags);
            open_table(pdpt.add(l3_idx), flags);
            open_table(pd.add(l2_idx), flags);
        }
        let phys_addr = pte_to_paddr(pte);
        let new_pte = make_leaf_entry(phys_addr, flags);
        unsafe {
            pt.add(l1_idx).write(new_pte);
        }

        self.flush_tlb_page(virt_addr);
        Ok(())
    }

    fn query(&self, virt_addr: usize) -> Option<(usize, MapFlags)> {
        if !is_canonical(virt_addr) {
            return None;
        }
        let indices = [
            pml4_index(virt_addr),
            pdpt_index(virt_addr),
            pd_index(virt_addr),
            pt_index(virt_addr),
        ];
        let mut table = self.root_paddr;
        let mut flags = MapFlags {
            readable: true,
            writable: true,
            executable: true,
            user: true,
        };
        for (depth, index) in indices.into_iter().enumerate() {
            let entry = unsafe { (table as *const u64).add(index).read() };
            if entry & PTE_PRESENT == 0 {
                return None;
            }
            flags.writable &= entry & PTE_WRITABLE != 0;
            flags.user &= entry & PTE_USER != 0;
            flags.executable &= entry & PTE_NO_EXECUTE == 0;
            if depth == 3 || ((depth == 1 || depth == 2) && entry & PTE_HUGE != 0) {
                let mask = (1usize << (12 + (3 - depth) * 9)) - 1;
                return Some(((pte_to_paddr(entry) & !mask) | (virt_addr & mask), flags));
            }
            table = pte_to_paddr(entry);
        }
        None
    }

    unsafe fn activate(&self) {
        core::arch::asm!(
            "mov cr3, {}",
            in(reg) self.root_paddr,
            options(nostack)
        );
    }

    unsafe fn flush_tlb_page(&self, virt_addr: usize) {
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) virt_addr,
            options(nostack)
        );
    }

    unsafe fn flush_tlb_all(&self) {
        core::arch::asm!(
            "mov rax, cr3",
            "mov cr3, rax",
            out("rax") _,
            options(nostack)
        );
    }
}

impl Drop for Pml4AddressSpace {
    fn drop(&mut self) {
        // Only lower tables are owned; upper tables and every mapped leaf belong elsewhere.
        unsafe {
            free_table(self.root_paddr, 4, 256);
        }
    }
}

/// Boot assertions for inherited permissions, huge leaves, and private table ownership.
pub fn self_test() {
    let before = bitmap::stats().free_pages;
    {
        let mut space = Pml4AddressSpace::new().expect("paging test root");
        let base = 0x0000_0030_0000_0000;
        unsafe {
            space.map_page(base, 0x1000, MapFlags::user_rw()).unwrap();
            space
                .map_page(base + 4096, 0x2000, MapFlags::user_rx())
                .unwrap();
            let root_slot = (space.root_paddr as *mut u64).add(pml4_index(base));
            let pdpt = pte_to_paddr(root_slot.read()) as *mut u64;
            let parent = pdpt.add(pdpt_index(base));
            let original = parent.read();
            parent.write((original & !(PTE_USER | PTE_WRITABLE)) | PTE_NO_EXECUTE);
            let restricted = space.query(base + 17).unwrap();
            assert_eq!(restricted.0, 0x1011);
            assert!(!restricted.1.user && !restricted.1.writable && !restricted.1.executable);
            // Opening a parent for one leaf must preserve sibling restrictions.
            space.protect_page(base, MapFlags::user_rw()).unwrap();
            assert_eq!(space.query(base).unwrap().1, MapFlags::user_rw());
            let sibling = space.query(base + 4096).unwrap().1;
            assert!(!sibling.user && !sibling.writable && !sibling.executable);

            let pd = pte_to_paddr(parent.read()) as *mut u64;
            pd.add(1)
                .write(0x20_0000 | PTE_PRESENT | PTE_USER | PTE_WRITABLE | PTE_HUGE);
            assert_eq!(space.query(base + 0x20_0123).unwrap().0, 0x20_0123);
            parent.write(parent.read() & !PTE_WRITABLE);
            assert!(!space.query(base + 0x20_0123).unwrap().1.writable);

            pdpt.add(pdpt_index(base) + 1)
                .write(0x4000_0000 | PTE_PRESENT | PTE_USER | PTE_WRITABLE | PTE_HUGE);
            assert_eq!(space.query(base + 0x4000_0123).unwrap().0, 0x4000_0123);
            root_slot.write(root_slot.read() & !PTE_USER);
            assert!(!space.query(base + 0x4000_0123).unwrap().1.user);
            assert_eq!(
                space.map_page(0xffff_8000_0000_0000, 0x3000, MapFlags::user_rw()),
                Err(PageError::OutOfRange)
            );
            let source = BOOT_ROOT.load(core::sync::atomic::Ordering::SeqCst) as *const u64;
            let source_pdpt = pte_to_paddr(source.add(pml4_index(base)).read()) as *const u64;
            assert_eq!(
                source_pdpt.add(pdpt_index(base)).read() & PTE_PRESENT,
                0,
                "private mapping modified boot root"
            );
        }
    }
    assert_eq!(
        bitmap::stats().free_pages,
        before,
        "paging self-test leaked tables"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_addresses() {
        // User space canonical bounds
        assert!(is_canonical(0x0000_0000_0000_0000));
        assert!(is_canonical(0x0000_7FFF_FFFF_FFFF));

        // Kernel space canonical bounds
        assert!(is_canonical(0xFFFF_8000_0000_0000));
        assert!(is_canonical(0xFFFF_FFFF_FFFF_FFFF));

        // Non-canonical gap
        assert!(!is_canonical(0x0000_8000_0000_0000));
        assert!(!is_canonical(0xFFFF_7FFF_FFFF_FFFF));
        assert!(!is_canonical(0x0000_8000_0000_0001));
    }

    #[test]
    fn test_4level_indices() {
        let va_user_start = 0x0000_0000_0000_0000;
        assert_eq!(pml4_index(va_user_start), 0);
        assert_eq!(pdpt_index(va_user_start), 0);
        assert_eq!(pd_index(va_user_start), 0);
        assert_eq!(pt_index(va_user_start), 0);
        assert_eq!(page_offset(va_user_start), 0);

        let va_user_end = 0x0000_7FFF_FFFF_F123;
        assert_eq!(pml4_index(va_user_end), 255);
        assert_eq!(pdpt_index(va_user_end), 511);
        assert_eq!(pd_index(va_user_end), 511);
        assert_eq!(pt_index(va_user_end), 511);
        assert_eq!(page_offset(va_user_end), 0x123);

        let va_kernel_start = 0xFFFF_8000_0000_0000;
        assert_eq!(pml4_index(va_kernel_start), 256);
        assert_eq!(pdpt_index(va_kernel_start), 0);
        assert_eq!(pd_index(va_kernel_start), 0);
        assert_eq!(pt_index(va_kernel_start), 0);
        assert_eq!(page_offset(va_kernel_start), 0);

        let va_kernel_end = 0xFFFF_FFFF_FFFF_FFFF;
        assert_eq!(pml4_index(va_kernel_end), 511);
        assert_eq!(pdpt_index(va_kernel_end), 511);
        assert_eq!(pd_index(va_kernel_end), 511);
        assert_eq!(pt_index(va_kernel_end), 511);
        assert_eq!(page_offset(va_kernel_end), 0xFFF);
    }

    #[test]
    fn test_pte_flags_encoding() {
        let phys = 0x0000_0000_1234_5000;
        let flags = MapFlags {
            readable: true,
            writable: true,
            executable: false,
            user: true,
        };
        let pte = make_leaf_entry(phys, flags);
        assert_ne!(pte & PTE_PRESENT, 0);
        assert_ne!(pte & PTE_WRITABLE, 0);
        assert_ne!(pte & PTE_USER, 0);
        assert_ne!(pte & PTE_NO_EXECUTE, 0);
        assert_ne!(pte & PTE_ACCESSED, 0);
        assert_ne!(pte & PTE_DIRTY, 0);
        assert_eq!(pte_to_paddr(pte), phys);

        let rx_flags = MapFlags {
            readable: true,
            writable: false,
            executable: true,
            user: false,
        };
        let rx_pte = make_leaf_entry(phys, rx_flags);
        assert_ne!(rx_pte & PTE_PRESENT, 0);
        assert_eq!(rx_pte & PTE_WRITABLE, 0);
        assert_eq!(rx_pte & PTE_USER, 0);
        assert_eq!(rx_pte & PTE_NO_EXECUTE, 0);
        assert_eq!(pte_to_paddr(rx_pte), phys);
    }
}
