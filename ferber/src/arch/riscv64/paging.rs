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

use crate::kernel::memory::{bitmap, phys_map};
use crate::kernel::memory::accounting::{self, AllocError, BudgetRef, FrameCharge};
use alloc::vec::Vec;
use crate::kernel::memory::vmo::VmoFrames;
use core::sync::atomic::{AtomicUsize, Ordering};
static KERNEL_ROOT: AtomicUsize = AtomicUsize::new(0);

/// Switch to the retained permanent root before reclaiming a task's page tables.
///
/// # Safety
/// The permanent root must be installed and the current stack supervisor-mapped.
#[cfg(not(test))]
pub unsafe fn activate_kernel_address_space() {
    let root = KERNEL_ROOT.load(Ordering::Acquire);
    assert_ne!(root, 0, "permanent kernel root unavailable");
    core::arch::asm!(
        "csrw satp, {satp}",
        "sfence.vma zero, zero",
        satp = in(reg) ((8usize << 60) | (root >> 12)),
        options(nostack)
    );
}

fn table(paddr: usize) -> *mut u64 {
    phys_map::phys_to_virt(paddr).expect("page table physical alias") as *mut u64
}
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
fn accounting_error(error: AllocError) -> PageError {
    match error { AllocError::Quota => PageError::JobQuota, AllocError::OutOfMemory => PageError::OutOfMemory }
}

fn allocate_table(charge: &mut FrameCharge) -> Result<usize, PageError> {
    charge.grow(1).map_err(accounting_error)?;
    let Some(paddr) = bitmap::allocate_frame() else {
        charge.release(1);
        return Err(PageError::OutOfMemory);
    };
    unsafe {
        core::ptr::write_bytes(table(paddr).cast::<u8>(), 0, PAGE_SIZE);
    }
    Ok(paddr)
}

fn is_table_pte(pte: u64) -> bool {
    pte & PTE_V != 0 && pte & !((PPN_MASK << 10) | PTE_V | PTE_G) == 0
}

fn is_leaf_pte(pte: u64) -> bool {
    pte & PTE_V != 0 && pte & (PTE_R | PTE_X) != 0
        && (pte & PTE_W == 0 || pte & PTE_R != 0)
        && pte & !((PPN_MASK << 10) | 0x3ff) == 0
}

fn validate_frames(frames: &VmoFrames, first: usize, count: usize) -> Result<(), PageError> {
    let end = first.checked_add(count).filter(|end| *end <= frames.count())
        .ok_or(PageError::OutOfRange)?;
    for i in first..end {
        let pa = frames.get(i).ok_or(PageError::OutOfRange)?;
        if !pa.is_multiple_of(PAGE_SIZE) { return Err(PageError::Unaligned); }
        if pa >> 56 != 0 { return Err(PageError::OutOfRange); }
    }
    Ok(())
}

struct StagedTables(Vec<(*mut u64, usize)>, FrameCharge);
impl Drop for StagedTables {
    fn drop(&mut self) {
        for &(_, pa) in &self.0 { let _ = bitmap::free_frame(pa); }
    }
}

/// Sv39 page table address space.
#[derive(Debug)]
pub struct Sv39AddressSpace {
    root_paddr: usize,
    table_pages: usize,
    table_limit: usize,
    charge: FrameCharge,
}

impl Sv39AddressSpace {
    /// Creates a new Sv39 address space by allocating a root Level 2 page table.
    pub fn new() -> Result<Self, PageError> {
        Self::with_owner(BudgetRef::system())
    }

    fn with_owner(owner: BudgetRef) -> Result<Self, PageError> {
        let mut charge = FrameCharge::reserve(&owner, 0).map_err(accounting_error)?;
        let root_paddr = allocate_table(&mut charge)?;
        let kernel_root = KERNEL_ROOT.load(Ordering::Acquire);
        if kernel_root != 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    table(kernel_root).add(256),
                    table(root_paddr).add(256),
                    256,
                );
            }
        }
        Ok(Self { root_paddr, table_pages: 1, table_limit: usize::MAX, charge })
    }

    pub fn new_user() -> Result<Self, PageError> {
        let mut space = Self::with_owner(accounting::current())?;
        space.table_limit = 256;
        Ok(space)
    }

    pub fn table_pages(&self) -> usize { self.table_pages }

    pub fn budget(&self) -> &BudgetRef { self.charge.owner() }

    fn validate_range(&self, va: usize, count: usize) -> Result<(), PageError> {
        if !va.is_multiple_of(PAGE_SIZE) { return Err(PageError::Unaligned); }
        let end = count.checked_mul(PAGE_SIZE).and_then(|bytes| va.checked_add(bytes))
            .ok_or(PageError::OutOfRange)?;
        if !is_canonical(va) || (count != 0 && (!is_canonical(end - 1)
            || vpn2(va) / 256 != vpn2(end - 1) / 256))
            || (vpn2(va) >= 256 && (self.table_limit != usize::MAX
                || KERNEL_ROOT.load(Ordering::Acquire) != 0)) {
            return Err(PageError::OutOfRange);
        }
        Ok(())
    }

    fn preflight_free_range(&self, va: usize, count: usize) -> Result<usize, PageError> {
        if count == 0 { return Err(PageError::OutOfRange); }
        self.validate_range(va, count)?;
        unsafe {
            let mut missing = 0usize;
            let mut previous = [None, None];
            for i in 0..count {
                let address = va + i * PAGE_SIZE;
                let mut current = Some(self.root_paddr);
                for (depth, index) in [vpn2(address), vpn1(address)].into_iter().enumerate() {
                    let pte = current.map(|pa| table(pa).add(index).read()).unwrap_or(0);
                    if pte == 0 {
                        let key = address >> (30 - depth * 9);
                        if previous[depth] != Some(key) { missing += 1; previous[depth] = Some(key); }
                        current = None;
                    } else if is_table_pte(pte) {
                        current = Some(pte_to_paddr(pte));
                    } else { return Err(PageError::AlreadyMapped); }
                }
                if let Some(pa) = current {
                    if table(pa).add(vpn0(address)).read() != 0 { return Err(PageError::AlreadyMapped); }
                }
            }
            Ok(missing)
        }
    }

    fn leaf_slot(&self, va: usize) -> Result<*mut u64, PageError> {
        let mut current = self.root_paddr;
        for index in [vpn2(va), vpn1(va)] {
            let pte = unsafe { table(current).add(index).read() };
            if !is_table_pte(pte) { return Err(PageError::NotMapped); }
            current = pte_to_paddr(pte);
        }
        let slot = unsafe { table(current).add(vpn0(va)) };
        if !is_leaf_pte(unsafe { slot.read() }) { return Err(PageError::NotMapped); }
        Ok(slot)
    }

    unsafe fn remove_leaf(&mut self, va: usize) -> usize {
        let root_slot = table(self.root_paddr).add(vpn2(va));
        let l1_pa = pte_to_paddr(root_slot.read());
        let l1_slot = table(l1_pa).add(vpn1(va));
        let l0_pa = pte_to_paddr(l1_slot.read());
        let leaf = table(l0_pa).add(vpn0(va));
        let pa = pte_to_paddr(leaf.read());
        leaf.write(0);
        let empty0 = (0..512).all(|i| table(l0_pa).add(i).read() == 0);
        // Upper-half tables belong to the permanent root and are never reclaimed here.
        let owned = vpn2(va) < 256;
        let mut empty1 = false;
        if owned && empty0 {
            l1_slot.write(0);
            empty1 = (0..512).all(|i| table(l1_pa).add(i).read() == 0);
            if empty1 { root_slot.write(0); }
        }
        self.flush_tlb_all();
        if owned && empty0 {
            bitmap::free_frame(l0_pa).expect("owned page table frame");
            self.charge.release(1);
            self.table_pages -= 1;
            if empty1 {
                bitmap::free_frame(l1_pa).expect("owned page table frame");
                self.charge.release(1);
                self.table_pages -= 1;
            }
        }
        pa
    }

    /// Returns the physical address of the root Level 2 table.
    pub fn root_paddr(&self) -> usize {
        self.root_paddr
    }
}

impl AddressSpace for Sv39AddressSpace {
    fn check_free_range(&self, va: usize, count: usize) -> Result<(), PageError> {
        if vpn2(va) >= 256 { return Err(PageError::OutOfRange); }
        self.preflight_free_range(va, count).map(|_| ())
    }

    unsafe fn map_page(&mut self, va: usize, pa: usize, flags: MapFlags) -> Result<(), PageError> {
        self.map_pages(va, &VmoFrames::contiguous(pa, 1), 0, 1, flags)
    }

    unsafe fn map_device_pages(&mut self, va: usize, physical: usize, count: usize, flags: MapFlags) -> Result<(), PageError> {
        if flags.executable || !flags.readable || count == 0 || count.checked_mul(PAGE_SIZE).and_then(|n| physical.checked_add(n)).is_none() {
            return Err(PageError::OutOfRange);
        }
        // QEMU virt UART/PLIC attributes come from platform PMAs, not Svpbmt.
        self.map_pages(va, &VmoFrames::contiguous(physical, count), 0, count, flags)
    }

    unsafe fn map_pages(&mut self, va: usize, frames: &VmoFrames, first: usize,
        count: usize, flags: MapFlags) -> Result<(), PageError> {
        self.validate_range(va, count)?;
        validate_frames(frames, first, count)?;
        if !flags.readable && !flags.writable && !flags.executable {
            return Err(PageError::OutOfRange);
        }
        let missing = self.preflight_free_range(va, count)?;
        if missing > self.table_limit - self.table_pages { return Err(PageError::TableBudget); }
        let mut staged = StagedTables(Vec::new(), FrameCharge::reserve(self.budget(), 0).map_err(accounting_error)?);
        accounting::with_budget(self.budget(), || accounting::attempt(|| staged.0.try_reserve_exact(missing)))
            .map_err(accounting_error)?;
        for i in 0..count {
            let address = va + i * PAGE_SIZE;
            let mut current = self.root_paddr;
            for index in [vpn2(address), vpn1(address)] {
                let slot = table(current).add(index);
                let pte = slot.read();
                current = if pte != 0 { pte_to_paddr(pte) }
                else if let Some(entry) = staged.0.iter().find(|entry| entry.0 == slot) { entry.1 }
                else {
                    let pa = allocate_table(&mut staged.1)?;
                    staged.0.push((slot, pa));
                    pa
                };
            }
        }
        // Every allocation and validation is complete before publishing a table or leaf.
        for &(slot, pa) in &staged.0 { slot.write(make_table_pte(pa)); }
        self.table_pages += staged.0.len();
        staged.0.clear();
        self.charge.merge(core::mem::replace(&mut staged.1,
            FrameCharge::reserve(self.budget(), 0).expect("zero table charge")));
        for i in 0..count {
            let address = va + i * PAGE_SIZE;
            let l1 = pte_to_paddr(table(self.root_paddr).add(vpn2(address)).read());
            let l0 = pte_to_paddr(table(l1).add(vpn1(address)).read());
            table(l0).add(vpn0(address)).write(make_leaf_pte(frames.get(first + i).unwrap(), flags));
        }
        self.flush_tlb_all();
        if flags.executable {
            // Translation fences do not publish prior RAM-alias writes to instruction fetch.
            #[cfg(test)]
            crate::paging_test_publish(self, va, count);
            #[cfg(not(test))]
            core::arch::asm!("fence.i", options(nostack));
        }
        Ok(())
    }

    unsafe fn unmap_pages_exact(&mut self, va: usize, frames: &VmoFrames, first: usize,
        count: usize, flags: MapFlags) -> Result<(), PageError> {
        self.validate_range(va, count)?;
        validate_frames(frames, first, count)?;
        for i in 0..count {
            let address = va + i * PAGE_SIZE;
            let pte = self.leaf_slot(address)?.read();
            let expected = make_leaf_pte(frames.get(first + i).unwrap(), flags);
            if pte_to_paddr(pte) != pte_to_paddr(expected)
                || pte & (PTE_R | PTE_W | PTE_X | PTE_U) != expected & (PTE_R | PTE_W | PTE_X | PTE_U) {
                return Err(PageError::NotMapped);
            }
        }
        for i in 0..count { self.remove_leaf(va + i * PAGE_SIZE); }
        Ok(())
    }

    unsafe fn unmap_page(&mut self, va: usize) -> Result<usize, PageError> {
        self.validate_range(va, 1)?;
        self.leaf_slot(va)?;
        Ok(self.remove_leaf(va))
    }

    unsafe fn protect_page(&mut self, virt_addr: usize, flags: MapFlags) -> Result<(), PageError> {
        self.validate_range(virt_addr, 1)?;

        // Level 2 walk
        let l2_table = table(self.root_paddr);
        let idx2 = vpn2(virt_addr);
        let pte2 = unsafe { l2_table.add(idx2).read() };
        if (pte2 & PTE_V) == 0 || (pte2 & (PTE_R | PTE_W | PTE_X)) != 0 {
            return Err(PageError::NotMapped);
        }
        let l1_table = table(pte_to_paddr(pte2));

        // Level 1 walk
        let idx1 = vpn1(virt_addr);
        let pte1 = unsafe { l1_table.add(idx1).read() };
        if (pte1 & PTE_V) == 0 || (pte1 & (PTE_R | PTE_W | PTE_X)) != 0 {
            return Err(PageError::NotMapped);
        }
        let l0_table = table(pte_to_paddr(pte1));

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
        let l2_table = table(self.root_paddr);
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
        let l1_table = table(pte_to_paddr(pte2));

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
        let l0_table = table(pte_to_paddr(pte1));

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
        #[cfg(not(test))]
        let satp = (8usize << 60) | (self.root_paddr >> 12);
        #[cfg(not(test))]
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma zero, zero",
            satp = in(reg) satp,
            options(nostack)
        );
    }

    unsafe fn flush_tlb_page(&self, virt_addr: usize) {
        #[cfg(test)]
        { let _ = virt_addr; crate::paging_test_flush(); }
        #[cfg(not(test))]
        core::arch::asm!(
            "sfence.vma {va}, zero",
            va = in(reg) virt_addr,
            options(nostack)
        );
    }

    unsafe fn flush_tlb_all(&self) {
        #[cfg(test)]
        crate::paging_test_flush();
        #[cfg(not(test))]
        core::arch::asm!("sfence.vma zero, zero", options(nostack));
    }
}

impl Drop for Sv39AddressSpace {
    fn drop(&mut self) {
        if self.root_paddr == 0 {
            return;
        }

        let l2_table = table(self.root_paddr);
        for i2 in 0..256 {
            let pte2 = unsafe { l2_table.add(i2).read() };
            if is_table_pte(pte2) {
                let l1_paddr = pte_to_paddr(pte2);
                let l1_table = table(l1_paddr);
                for i1 in 0..ENTRIES_PER_TABLE {
                    let pte1 = unsafe { l1_table.add(i1).read() };
                    if is_table_pte(pte1) {
                        let l0_paddr = pte_to_paddr(pte1);
                        bitmap::free_frame(l0_paddr).expect("owned page table frame");
                    }
                }
                bitmap::free_frame(l1_paddr).expect("owned page table frame");
            }
        }
        bitmap::free_frame(self.root_paddr).expect("owned page table root");
        self.charge.release(self.charge.pages());
        self.root_paddr = 0;
    }
}

#[cfg(not(test))]
#[repr(C, align(4096))]
struct BootstrapTables([[u64; ENTRIES_PER_TABLE]; 248]);
#[cfg(not(test))]
static mut BOOTSTRAP_TABLES: BootstrapTables = BootstrapTables([[0; ENTRIES_PER_TABLE]; 248]);

#[cfg(not(test))]
pub fn physical_kernel_bounds() -> (usize, usize) {
    extern "C" {
        static riscv_kernel_physical_bounds: [usize; 2];
    }
    unsafe {
        (
            riscv_kernel_physical_bounds[0],
            riscv_kernel_physical_bounds[1],
        )
    }
}

#[cfg(not(test))]
pub fn runtime_physical_base() -> usize {
    extern "C" {
        static riscv_runtime_physical_base: usize;
    }
    unsafe { riscv_runtime_physical_base }
}

/// Install temporary large-page aliases before the allocator can supply tables.
/// Edge padding is removed when the permanent root replaces this bootstrap root.
///
/// # Safety
/// Invoke once, after physical-map discovery and before accessing allocator metadata.
#[cfg(not(test))]
pub unsafe fn bootstrap_aliases() {
    extern "C" {
        static riscv_boot_root_physical: usize;
    }
    let root = riscv_boot_root_physical as *mut u64;
    let tables = core::ptr::addr_of_mut!(BOOTSTRAP_TABLES).cast::<[u64; ENTRIES_PER_TABLE]>();
    let physical =
        tables as usize - crate::boot_info::KERNEL_IMAGE_BASE as usize + runtime_physical_base();
    for index in 0..248 {
        root.add(256 + index)
            .write(make_table_pte(physical + index * PAGE_SIZE));
    }
    for extent in phys_map::extents() {
        assert!(
            extent.end <= (1usize << 56),
            "RAM exceeds Sv39 physical address width"
        );
        let first = extent.virtual_start & !((1 << 21) - 1);
        let end =
            (extent.virtual_start + extent.end - extent.start + (1 << 21) - 1) & !((1 << 21) - 1);
        let mut physical = extent.start & !((1 << 21) - 1);
        let mut virtual_address = first;
        while virtual_address < end {
            let index = vpn2(virtual_address) - 256;
            (*tables.add(index))[vpn1(virtual_address)] =
                make_leaf_pte(physical, MapFlags::kernel_rw());
            physical += 1 << 21;
            virtual_address += 1 << 21;
        }
    }
    core::arch::asm!("sfence.vma zero, zero", options(nostack));
}

#[cfg(not(test))]
impl Sv39AddressSpace {
    unsafe fn map_large(&mut self, va: usize, pa: usize, flags: MapFlags) -> Result<(), PageError> {
        let root = table(self.root_paddr);
        let slot = root.add(vpn2(va));
        let mut entry = slot.read();
        if entry & PTE_V == 0 {
            entry = make_table_pte(allocate_table(&mut self.charge)?);
            self.table_pages += 1;
            slot.write(entry);
        }
        if entry & (PTE_R | PTE_W | PTE_X) != 0 {
            return Err(PageError::AlreadyMapped);
        }
        let slot = table(pte_to_paddr(entry)).add(vpn1(va));
        if slot.read() & PTE_V != 0 {
            return Err(PageError::AlreadyMapped);
        }
        slot.write(make_leaf_pte(pa, flags));
        Ok(())
    }
}

/// Replace temporary boot translations with exact permanent supervisor mappings.
///
/// # Safety
/// The allocator and physical aliases must already be initialized. Invoke once.
#[cfg(not(test))]
pub unsafe fn finalize_kernel_space(info: &crate::boot_info::BootInfo) {
    extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
        static __text_end: u8;
        static __rodata_end: u8;
    }
    let mut space = Sv39AddressSpace::new().expect("permanent kernel root");
    let start = core::ptr::addr_of!(__kernel_start) as usize;
    let end = core::ptr::addr_of!(__kernel_end) as usize;
    let text_end = core::ptr::addr_of!(__text_end) as usize;
    let rodata_end = core::ptr::addr_of!(__rodata_end) as usize;
    for va in (start..end).step_by(PAGE_SIZE) {
        let flags = if va < text_end {
            MapFlags::kernel_rx()
        } else if va < rodata_end {
            MapFlags::kernel_ro()
        } else {
            MapFlags::kernel_rw()
        };
        space
            .map_page(va, runtime_physical_base() + (va - start), flags)
            .expect("kernel image mapping");
    }
    for extent in phys_map::extents() {
        let mut pa = extent.start;
        let mut va = extent.virtual_start;
        while pa < extent.end {
            if pa.is_multiple_of(1 << 21)
                && va.is_multiple_of(1 << 21)
                && extent.end - pa >= 1 << 21
            {
                space
                    .map_large(va, pa, MapFlags::kernel_rw())
                    .expect("large RAM alias");
                pa += 1 << 21;
                va += 1 << 21;
            } else {
                space
                    .map_page(va, pa, MapFlags::kernel_rw())
                    .expect("RAM alias");
                pa += PAGE_SIZE;
                va += PAGE_SIZE;
            }
        }
    }
    if let Some((start, end)) = info.boot_payload.bounds().expect("boot payload bounds") {
        let base = crate::boot_info::payload::PAYLOAD_BASE as usize;
        for pa in (start as usize..end as usize).step_by(PAGE_SIZE) {
            space
                .map_page(base + pa - start as usize, pa, MapFlags::kernel_ro())
                .expect("boot payload mapping");
        }
    }
    KERNEL_ROOT.store(space.root_paddr, Ordering::Release);
    space.activate();
    core::mem::forget(space);
}
