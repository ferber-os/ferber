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

use alloc::vec::Vec;
use crate::kernel::memory::vmo::VmoFrames;
use crate::boot_info::{BootInfo, BOOT_INFO_VA, BOOT_REGIONS_VA, MMIO_BASE};
use crate::kernel::memory::paging::{AddressSpace, MapFlags, PageError};
use crate::kernel::memory::{bitmap, phys_map};

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
        core::ptr::write_bytes(table_ptr(paddr).cast::<u8>(), 0, PAGE_SIZE);
    }
    Ok(paddr)
}

fn table_ptr(physical: usize) -> *mut u64 {
    phys_map::phys_to_virt(physical).expect("page table outside RAM aliases") as *mut u64
}

fn physical_bits() -> u32 {
    let extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if extended >= 0x8000_0008 { (core::arch::x86_64::__cpuid(0x8000_0008).eax & 255).min(52) } else { 36 }
}

struct PendingTables(Vec<usize>);
impl Drop for PendingTables {
    fn drop(&mut self) {
        for pa in self.0.drain(..) { bitmap::free_frame(pa).expect("unpublished table frame"); }
    }
}

/// x86-64 PML4 page table address space.
#[derive(Debug)]
pub struct Pml4AddressSpace {
    root_paddr: usize,
    table_pages: usize,
    table_limit: usize,
}

static BOOT_ROOT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Restore the permanent supervisor root before reclaiming a task's tables.
///
/// # Safety
/// Execution must be on a retained supervisor stack mapped by the permanent root.
pub unsafe fn activate_kernel_address_space() {
    let root = BOOT_ROOT.load(core::sync::atomic::Ordering::SeqCst);
    assert_ne!(root, 0, "permanent root not initialized");
    core::arch::asm!("mov cr3, {}", in(reg) root, options(nostack));
}

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

const SCRATCH_TABLE: usize = crate::boot_info::BOOT_DATA_BASE as usize + 0x20_0000;
const SCRATCH: usize = crate::boot_info::BOOT_DATA_BASE as usize + 0x40_0000;

unsafe fn scratch(physical: usize) -> *mut u64 {
    (SCRATCH_TABLE as *mut u64)
        .write_volatile(physical as u64 | PTE_PRESENT | PTE_WRITABLE | PTE_NO_EXECUTE);
    core::arch::asm!("invlpg [{}]", in(reg) SCRATCH, options(nostack));
    SCRATCH as *mut u64
}

/// Populate packed aliases without dereferencing a physical address directly.
/// Each scratch pointer is used immediately, before mapping another page.
pub fn bootstrap_aliases(_info: &BootInfo) -> Result<(), PageError> {
    let root = BOOT_ROOT.load(core::sync::atomic::Ordering::SeqCst);
    assert_ne!(root, 0, "loader root not captured");
    let extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    let physical_bits = if extended >= 0x8000_0008 {
        core::arch::x86_64::__cpuid(0x8000_0008).eax & 255
    } else {
        36
    };
    for extent in phys_map::extents() {
        if extent.end as u64 > (1u64 << physical_bits.min(52)) {
            return Err(PageError::OutOfRange);
        }
        let mut pa = extent.start;
        let mut va = extent.virtual_start;
        while pa < extent.end {
            let huge =
                (pa | va).is_multiple_of(2 * 1024 * 1024) && extent.end - pa >= 2 * 1024 * 1024;
            let indices = [pml4_index(va), pdpt_index(va), pd_index(va), pt_index(va)];
            let depth = if huge { 2 } else { 3 };
            let mut table = root;
            unsafe {
                for index in &indices[..depth] {
                    let mut entry = scratch(table).add(*index).read_volatile();
                    if entry == 0 {
                        let child =
                            phys_map::bootstrap_allocate_page().ok_or(PageError::OutOfMemory)?;
                        core::ptr::write_bytes(scratch(child).cast::<u8>(), 0, PAGE_SIZE);
                        entry = child as u64 | PTE_PRESENT | PTE_WRITABLE;
                        scratch(table).add(*index).write_volatile(entry);
                    }
                    assert_eq!(
                        entry & PTE_HUGE,
                        0,
                        "bootstrap alias collides with huge leaf"
                    );
                    table = pte_to_paddr(entry);
                }
                let slot = scratch(table).add(indices[depth]);
                assert_eq!(slot.read_volatile(), 0, "bootstrap alias overlap");
                slot.write_volatile(
                    pa as u64
                        | PTE_PRESENT
                        | PTE_WRITABLE
                        | PTE_NO_EXECUTE
                        | if huge { PTE_HUGE } else { 0 },
                );
            }
            let step = if huge { 2 * 1024 * 1024 } else { PAGE_SIZE };
            pa += step;
            va += step;
        }
    }
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) root, options(nostack));
    }
    Ok(())
}

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
}

unsafe fn map_kernel_leaf(
    root: usize,
    va: usize,
    pa: usize,
    flags: u64,
    huge: bool,
) -> Result<(), PageError> {
    let indices = [pml4_index(va), pdpt_index(va), pd_index(va), pt_index(va)];
    let depth = if huge { 2 } else { 3 };
    let mut table = root;
    for index in &indices[..depth] {
        let slot = table_ptr(table).add(*index);
        if slot.read() == 0 {
            slot.write(allocate_table()? as u64 | PTE_PRESENT | PTE_WRITABLE);
        }
        assert_eq!(slot.read() & PTE_HUGE, 0, "overlapping permanent map");
        table = pte_to_paddr(slot.read());
    }
    let slot = table_ptr(table).add(indices[depth]);
    assert_eq!(slot.read(), 0, "duplicate permanent map");
    slot.write(pa as u64 | flags | if huge { PTE_HUGE } else { 0 });
    Ok(())
}

unsafe fn map_kernel_range(
    root: usize,
    mut va: usize,
    mut pa: usize,
    len: usize,
    flags: u64,
) -> Result<(), PageError> {
    let end = pa.checked_add(len).ok_or(PageError::OutOfRange)?;
    while pa < end {
        let huge = (va | pa).is_multiple_of(2 * 1024 * 1024) && end - pa >= 2 * 1024 * 1024;
        map_kernel_leaf(root, va, pa, flags, huge)?;
        let step = if huge { 2 * 1024 * 1024 } else { PAGE_SIZE };
        pa += step;
        va += step;
    }
    Ok(())
}

/// Replace the loader's identity map with owned, supervisor-only upper tables.
/// The entry stack, descriptors, and all code already reside in the high image.
pub fn finalize(info: &BootInfo) -> Result<(), PageError> {
    let root = allocate_table()?;
    let rw = PTE_PRESENT | PTE_WRITABLE | PTE_NO_EXECUTE;
    unsafe {
        for extent in phys_map::extents() {
            map_kernel_range(
                root,
                extent.virtual_start,
                extent.start,
                extent.end - extent.start,
                rw,
            )?;
        }
        let text_start = core::ptr::addr_of!(__text_start) as usize;
        let text_end = core::ptr::addr_of!(__text_end) as usize;
        let ro_start = core::ptr::addr_of!(__rodata_start) as usize;
        let ro_end = core::ptr::addr_of!(__rodata_end) as usize;
        let mut va = info.kernel_virtual_start as usize;
        let end = va + (info.kernel_end - info.kernel_start) as usize;
        while va < end {
            let flags = if (text_start..text_end).contains(&va) {
                PTE_PRESENT
            } else if (ro_start..ro_end).contains(&va) {
                PTE_PRESENT | PTE_NO_EXECUTE
            } else {
                rw
            };
            map_kernel_leaf(
                root,
                va,
                info.kernel_start as usize + (va - info.kernel_virtual_start as usize),
                flags,
                false,
            )?;
            va += PAGE_SIZE;
        }
        map_kernel_range(
            root,
            BOOT_INFO_VA as usize,
            info.boot_info_physical as usize,
            PAGE_SIZE,
            PTE_PRESENT | PTE_NO_EXECUTE,
        )?;
        let region_bytes = (info.memory_region_count as usize
            * core::mem::size_of::<crate::boot_info::MemoryRegion>())
        .div_ceil(PAGE_SIZE)
            * PAGE_SIZE;
        map_kernel_range(
            root,
            BOOT_REGIONS_VA as usize,
            info.memory_regions as usize,
            region_bytes,
            PTE_PRESENT | PTE_NO_EXECUTE,
        )?;
        if let Some((start, end)) = info
            .boot_payload
            .bounds()
            .map_err(|_| PageError::OutOfRange)?
        {
            map_kernel_range(
                root,
                (info.boot_payload.virtual_start - (info.boot_payload.physical_start - start))
                    as usize,
                start as usize,
                (end - start) as usize,
                PTE_PRESENT | PTE_NO_EXECUTE,
            )?;
        }
        if info.framebuffer.size != 0 {
            let offset = info.framebuffer.base as usize & (PAGE_SIZE - 1);
            let bytes = (offset + info.framebuffer.size as usize).div_ceil(PAGE_SIZE) * PAGE_SIZE;
            map_kernel_range(
                root,
                MMIO_BASE as usize,
                info.framebuffer.base as usize - offset,
                bytes,
                rw | PTE_PCD,
            )?;
        }
        core::arch::asm!("mov cr3, {}", in(reg) root, options(nostack));
    }
    phys_map::activate();
    BOOT_ROOT.store(root, core::sync::atomic::Ordering::SeqCst);
    Ok(())
}

unsafe fn free_table(table: usize, level: usize, count: usize) {
    for i in 0..count {
        let entry = (table_ptr(table) as *const u64).add(i).read();
        if level > 1 && entry & PTE_PRESENT != 0 && entry & PTE_HUGE == 0 {
            free_table(pte_to_paddr(entry), level - 1, 512);
        }
    }
    bitmap::free_frame(table).expect("owned page table frame");
}

// Preserve sibling restrictions before opening an ancestor for a user mapping.
unsafe fn open_table(slot: *mut u64, flags: MapFlags) {
    let mut entry = slot.read();
    let relax_user = flags.user && entry & PTE_USER == 0;
    let relax_write = flags.writable && entry & PTE_WRITABLE == 0;
    let relax_execute = flags.executable && entry & PTE_NO_EXECUTE != 0;
    if relax_user || relax_write || relax_execute {
        let child = table_ptr(pte_to_paddr(entry));
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
            table_pages: 1,
            table_limit: usize::MAX,
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                table_ptr(source).add(256),
                table_ptr(result.root_paddr).add(256),
                256,
            );
        }
        Ok(result)
    }

    pub fn new_user() -> Result<Self, PageError> {
        let mut result = Self::new()?;
        result.table_limit = 256;
        Ok(result)
    }

    pub fn table_pages(&self) -> usize { self.table_pages }

    fn validate_virtual_range(va: usize, count: usize) -> Result<(), PageError> {
        if !va.is_multiple_of(PAGE_SIZE) { return Err(PageError::Unaligned); }
        if count == 0 { return Err(PageError::OutOfRange); }
        let end = count.checked_mul(PAGE_SIZE).and_then(|bytes| va.checked_add(bytes - 1)).ok_or(PageError::OutOfRange)?;
        if !is_canonical(va) || !is_canonical(end) || pml4_index(va) >= 256 || pml4_index(end) >= 256 {
            return Err(PageError::OutOfRange);
        }
        Ok(())
    }

    fn preflight_free_range(&self, va: usize, count: usize) -> Result<usize, PageError> {
        Self::validate_virtual_range(va, count)?;
        unsafe {
            let mut needed = 0usize;
            let mut previous = [usize::MAX; 3];
            for page in 0..count {
                let addr = va + page * PAGE_SIZE;
                let indices = [pml4_index(addr), pdpt_index(addr), pd_index(addr), pt_index(addr)];
                let mut table = Some(self.root_paddr);
                for depth in 0..4 {
                    let entry = table.map(|p| table_ptr(p).add(indices[depth]).read()).unwrap_or(0);
                    if depth == 3 {
                        if entry != 0 { return Err(PageError::AlreadyMapped); }
                    } else if entry == 0 {
                        let prefix = addr >> (39 - depth * 9);
                        if previous[depth] != prefix { needed += 1; previous[depth] = prefix; }
                        table = None;
                    } else {
                        if entry & PTE_PRESENT == 0 || entry & PTE_HUGE != 0 { return Err(PageError::AlreadyMapped); }
                        table = Some(pte_to_paddr(entry));
                    }
                }
            }
            Ok(needed)
        }
    }

    fn validate_range(va: usize, frames: &VmoFrames, first: usize, count: usize, flags: MapFlags) -> Result<(), PageError> {
        Self::validate_virtual_range(va, count)?;
        if !flags.readable || first.checked_add(count).filter(|end| *end <= frames.count()).is_none() {
            return Err(PageError::OutOfRange);
        }
        let bits = physical_bits();
        for i in first..first + count {
            let pa = frames.get(i).ok_or(PageError::OutOfRange)?;
            if !pa.is_multiple_of(PAGE_SIZE) { return Err(PageError::Unaligned); }
            if pa as u64 >= (1u64 << bits) { return Err(PageError::OutOfRange); }
        }
        Ok(())
    }

    // The caller preflights the entire range before reaching this infallible walk.
    unsafe fn remove_leaf(&mut self, va: usize) -> usize {
        let indices = [pml4_index(va), pdpt_index(va), pd_index(va), pt_index(va)];
        let mut tables = [self.root_paddr; 4];
        for depth in 0..3 {
            tables[depth + 1] = pte_to_paddr(table_ptr(tables[depth]).add(indices[depth]).read());
        }
        let slot = table_ptr(tables[3]).add(indices[3]);
        let pa = pte_to_paddr(slot.read());
        slot.write(0);
        self.flush_tlb_page(va);
        for depth in (1..4).rev() {
            if (0..512).any(|i| table_ptr(tables[depth]).add(i).read() != 0) { break; }
            table_ptr(tables[depth - 1]).add(indices[depth - 1]).write(0);
            // Retiring a nonleaf invalidates paging-structure caches before reuse.
            self.flush_tlb_all();
            bitmap::free_frame(tables[depth]).expect("owned private page table");
            self.table_pages -= 1;
        }
        pa
    }

    pub fn root_paddr(&self) -> usize {
        self.root_paddr
    }
}

impl AddressSpace for Pml4AddressSpace {
    fn check_free_range(&self, va: usize, count: usize) -> Result<(), PageError> {
        self.preflight_free_range(va, count).map(|_| ())
    }

    unsafe fn map_page(
        &mut self,
        virt_addr: usize,
        phys_addr: usize,
        flags: MapFlags,
    ) -> Result<(), PageError> {
        self.map_pages(virt_addr, &VmoFrames::contiguous(phys_addr, 1), 0, 1, flags)
    }

    unsafe fn map_pages(&mut self, va: usize, frames: &VmoFrames, first: usize, count: usize, flags: MapFlags) -> Result<(), PageError> {
        Self::validate_range(va, frames, first, count, flags)?;
        let needed = self.preflight_free_range(va, count)?;
        if needed > self.table_limit - self.table_pages { return Err(PageError::TableBudget); }
        let mut pending = PendingTables(Vec::new());
        pending.0.try_reserve_exact(needed).map_err(|_| PageError::OutOfMemory)?;
        for _ in 0..needed { pending.0.push(allocate_table()?); }
        for page in 0..count {
            let addr = va + page * PAGE_SIZE;
            let indices = [pml4_index(addr), pdpt_index(addr), pd_index(addr), pt_index(addr)];
            let mut table = self.root_paddr;
            for index in &indices[..3] {
                let slot = table_ptr(table).add(*index);
                if slot.read() == 0 {
                    slot.write(make_table_entry(pending.0.pop().expect("preflight counted tables")));
                    self.table_pages += 1;
                } else { open_table(slot, flags); }
                table = pte_to_paddr(slot.read());
            }
            table_ptr(table).add(indices[3]).write(make_leaf_entry(frames.get(first + page).unwrap(), flags));
        }
        debug_assert!(pending.0.is_empty());
        self.flush_tlb_all();
        Ok(())
    }

    unsafe fn unmap_pages_exact(&mut self, va: usize, frames: &VmoFrames, first: usize, count: usize, flags: MapFlags) -> Result<(), PageError> {
        Self::validate_range(va, frames, first, count, flags)?;
        for page in 0..count {
            let addr = va + page * PAGE_SIZE;
            let indices = [pml4_index(addr), pdpt_index(addr), pd_index(addr), pt_index(addr)];
            let mut table = self.root_paddr;
            for (depth, index) in indices.into_iter().enumerate() {
                let entry = table_ptr(table).add(index).read();
                if entry & PTE_PRESENT == 0 || (depth < 3 && entry & PTE_HUGE != 0) { return Err(PageError::NotMapped); }
                table = pte_to_paddr(entry);
            }
            if self.query(addr) != Some((frames.get(first + page).unwrap(), flags)) { return Err(PageError::NotMapped); }
        }
        for page in 0..count { self.remove_leaf(va + page * PAGE_SIZE); }
        Ok(())
    }

    unsafe fn unmap_page(&mut self, va: usize) -> Result<usize, PageError> {
        if !va.is_multiple_of(PAGE_SIZE) { return Err(PageError::Unaligned); }
        if !is_canonical(va) || pml4_index(va) >= 256 { return Err(PageError::OutOfRange); }
        let indices = [pml4_index(va), pdpt_index(va), pd_index(va), pt_index(va)];
        let mut table = self.root_paddr;
        for (depth, index) in indices.into_iter().enumerate() {
            let entry = table_ptr(table).add(index).read();
            if entry & PTE_PRESENT == 0 || (depth < 3 && entry & PTE_HUGE != 0) { return Err(PageError::NotMapped); }
            table = pte_to_paddr(entry);
        }
        Ok(self.remove_leaf(va))
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
        let pml4 = table_ptr(self.root_paddr);
        let l4_idx = pml4_index(virt_addr);
        let pml4e = unsafe { pml4.add(l4_idx).read() };
        if (pml4e & PTE_PRESENT) == 0 {
            return Err(PageError::NotMapped);
        }
        let pdpt = table_ptr(pte_to_paddr(pml4e));

        // Level 3 (PDPT) walk
        let l3_idx = pdpt_index(virt_addr);
        let pdpte = unsafe { pdpt.add(l3_idx).read() };
        if (pdpte & PTE_PRESENT) == 0 || (pdpte & PTE_HUGE) != 0 {
            return Err(PageError::NotMapped);
        }
        let pd = table_ptr(pte_to_paddr(pdpte));

        // Level 2 (PD) walk
        let l2_idx = pd_index(virt_addr);
        let pde = unsafe { pd.add(l2_idx).read() };
        if (pde & PTE_PRESENT) == 0 || (pde & PTE_HUGE) != 0 {
            return Err(PageError::NotMapped);
        }
        let pt = table_ptr(pte_to_paddr(pde));

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
            let entry = unsafe { (table_ptr(table) as *const u64).add(index).read() };
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
        #[cfg(test)]
        { let _ = virt_addr; crate::test_flush(false); }
        #[cfg(not(test))]
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) virt_addr,
            options(nostack)
        );
    }

    unsafe fn flush_tlb_all(&self) {
        #[cfg(test)]
        crate::test_flush(true);
        #[cfg(not(test))]
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
            let root_slot = (table_ptr(space.root_paddr)).add(pml4_index(base));
            let pdpt = table_ptr(pte_to_paddr(root_slot.read()));
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

            let pd = table_ptr(pte_to_paddr(parent.read()));
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
            let source = table_ptr(BOOT_ROOT.load(core::sync::atomic::Ordering::SeqCst));
            assert_eq!(
                source.add(pml4_index(base)).read(),
                0,
                "private mapping modified permanent root"
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

#[cfg(test)]
mod transaction_tests {
    use super::*;

    #[test]
    fn production_range_transactions() {
        let boot = allocate_table().unwrap();
        BOOT_ROOT.store(boot, core::sync::atomic::Ordering::SeqCst);
        let flags = MapFlags::user_rw();
        let frames = VmoFrames::contiguous(0x1000, 2);
        for boundary in [1usize << 21, 1 << 30, 1 << 39] {
            let va = boundary - PAGE_SIZE;
            for fail in 0..=6 {
                let mut space = Pml4AddressSpace::new_user().unwrap();
                let before = bitmap::stats().free_pages;
                crate::mock_bitmap::STATE.lock().unwrap().1 = fail;
                let map_flushes = crate::mock_bitmap::STATE.lock().unwrap().2;
                let result = unsafe { space.map_pages(va, &frames, 0, 2, flags) };
                assert_eq!(crate::mock_bitmap::STATE.lock().unwrap().2,
                    map_flushes + usize::from(result.is_ok()));
                crate::mock_bitmap::STATE.lock().unwrap().1 = usize::MAX;
                if result.is_err() {
                    assert_eq!(bitmap::stats().free_pages, before);
                    assert_eq!(space.table_pages(), 1);
                    assert!((0..256).all(|i| unsafe { table_ptr(space.root_paddr).add(i).read() == 0 }));
                } else {
                    let before_flush = crate::mock_bitmap::STATE.lock().unwrap().2;
                    crate::mock_bitmap::EVENTS.lock().unwrap().clear();
                    unsafe { space.unmap_pages_exact(va, &frames, 0, 2, flags).unwrap(); }
                    let events = crate::mock_bitmap::EVENTS.lock().unwrap();
                    for (i, flush) in events.iter().enumerate() {
                        if !flush { assert!(i > 0 && events[i - 1], "table freed before full flush"); }
                    }
                    drop(events);
                    assert_eq!(space.table_pages(), 1);
                    assert_eq!(bitmap::stats().free_pages, before);
                    assert!(crate::mock_bitmap::STATE.lock().unwrap().2 > before_flush);
                }
            }
        }
        let mut space = Pml4AddressSpace::new_user().unwrap();
        unsafe {
            space.map_page(0, 0x1000, flags).unwrap();
            let root_slot = table_ptr(space.root_paddr);
            root_slot.write((root_slot.read() & !(PTE_USER | PTE_WRITABLE)) | PTE_NO_EXECUTE);
            let snapshot = (0..512).map(|i| root_slot.add(i).read()).collect::<Vec<_>>();
            assert_eq!(space.map_pages(0, &frames, 0, 2, flags), Err(PageError::AlreadyMapped));
            assert_eq!(snapshot, (0..512).map(|i| root_slot.add(i).read()).collect::<Vec<_>>());
            assert_eq!(space.unmap_pages_exact(0, &frames, 0, 2, flags), Err(PageError::NotMapped));
            assert!(space.query(0).is_some());
            // Raw non-present reservations at every level must survive rejection.
            for depth in 0..4 {
                let mut table = space.root_paddr;
                for _ in 0..depth { table = pte_to_paddr(table_ptr(table).read()); }
                let slot = table_ptr(table).add(1);
                let old = slot.read(); slot.write(0x200);
                let va = 1usize << (39 - depth * 9);
                let before = bitmap::stats().free_pages;
                space.table_limit = space.table_pages;
                assert_eq!(space.check_free_range(va, 1), Err(PageError::AlreadyMapped));
                assert_eq!(space.check_free_range(va, 0), Err(PageError::OutOfRange));
                assert_eq!(space.map_page(va, 0x2000, flags), Err(PageError::AlreadyMapped));
                space.table_limit = 256;
                assert_eq!(slot.read(), 0x200); assert_eq!(bitmap::stats().free_pages, before);
                slot.write(old);
            }
        }
        drop(space);
        // Fail after every off-tree allocation with restrictive existing ancestors.
        for fail in 0..3 {
            let mut space = Pml4AddressSpace::new_user().unwrap();
            unsafe {
                space.map_page(0, 0x1000, flags).unwrap();
                let root = table_ptr(space.root_paddr);
                root.write((root.read() & !(PTE_USER | PTE_WRITABLE)) | PTE_NO_EXECUTE);
                let mut saved = Vec::new();
                let mut table = space.root_paddr;
                for depth in 0..4 {
                    saved.push((table, (0..512).map(|i| table_ptr(table).add(i).read()).collect::<Vec<_>>()));
                    if depth < 3 { table = pte_to_paddr(table_ptr(table).read()); }
                }
                let before = bitmap::stats().free_pages;
                crate::mock_bitmap::STATE.lock().unwrap().1 = fail;
                let result = space.map_page(1 << 39, 0x2000, flags);
                crate::mock_bitmap::STATE.lock().unwrap().1 = usize::MAX;
                assert_eq!(result, Err(PageError::OutOfMemory));
                assert_eq!(bitmap::stats().free_pages, before);
                for (table, entries) in saved { assert_eq!(entries, (0..512).map(|i| table_ptr(table).add(i).read()).collect::<Vec<_>>()); }
                // Reject huge leaves rather than splitting them during a 4 KiB request.
                let pdpt = table_ptr(pte_to_paddr(root.read()));
                let old = pdpt.add(1).read();
                pdpt.add(1).write(0x4000_0000 | PTE_PRESENT | PTE_HUGE);
                assert_eq!(space.map_page(1 << 30, 0x3000, flags), Err(PageError::AlreadyMapped));
                assert_eq!(space.unmap_pages_exact(1 << 30, &frames, 0, 1, flags), Err(PageError::NotMapped));
                pdpt.add(1).write(old);
                let pd = table_ptr(pte_to_paddr(pdpt.read()));
                pd.add(1).write(0x20_0000 | PTE_PRESENT | PTE_HUGE);
                assert_eq!(space.map_page(1 << 21, 0x3000, flags), Err(PageError::AlreadyMapped));
                assert_eq!(space.unmap_page(1 << 21), Err(PageError::NotMapped));
                pd.add(1).write(0);
                // A software reservation keeps its table alive after the last leaf leaves.
                let pt = table_ptr(pte_to_paddr(pd.read()));
                pt.add(1).write(0x200);
                space.unmap_page(0).unwrap();
                assert_eq!(space.table_pages(), 4);
                assert_eq!(pt.add(1).read(), 0x200);
                pt.add(1).write(0);
                space.map_page(0, 0x1000, flags).unwrap();
                space.unmap_page(0).unwrap();
                assert_eq!(space.table_pages(), 1);
            }
        }
        let mut space = Pml4AddressSpace::new_user().unwrap();
        for i in 0..85 { unsafe { space.map_page(i << 39, 0x1000, flags).unwrap(); } }
        assert_eq!(space.table_pages(), 256);
        assert_eq!(unsafe { space.map_page(85 << 39, 0x1000, flags) }, Err(PageError::TableBudget));
        for i in 0..85 { unsafe { space.unmap_page(i << 39).unwrap(); } }
        assert_eq!(space.table_pages(), 1);
        for i in 0..300 { unsafe { space.map_page((i % 128) << 39, 0x1000, flags).unwrap(); space.unmap_page((i % 128) << 39).unwrap(); } }
        drop(space);
        bitmap::free_frame(boot).unwrap();
    }
}
