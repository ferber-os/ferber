use crate::{
    abi::{self, BootInfo},
    uefi, KernelImage, PAGE_SIZE,
};
use core::{arch::asm, ptr};

const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const CACHE_DISABLE: u64 = 1 << 4;
const HUGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const TWO_MIB: u64 = 2 * 1024 * 1024;
const IDENTITY_LIMIT: u64 = 1 << 47;

pub struct PageTables {
    pub root: u64,
    pub nx_supported: bool,
}

impl PageTables {
    unsafe fn new(services: *mut uefi::BootServices) -> Result<Self, uefi::Status> {
        let mut cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        if cr4 & (1 << 12) != 0 {
            return Err(uefi::UNSUPPORTED);
        } // LA57 needs a fifth level.
        let max = core::arch::x86_64::__cpuid(0x8000_0000).eax;
        let nx_supported =
            max >= 0x8000_0001 && core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20) != 0;
        let root = allocate_table(services)?;
        Ok(Self { root, nx_supported })
    }

    fn flags(&self, writable: bool, executable: bool, uncached: bool) -> u64 {
        PRESENT
            | if writable { WRITABLE } else { 0 }
            | if uncached { CACHE_DISABLE } else { 0 }
            | if self.nx_supported && !executable {
                NO_EXECUTE
            } else {
                0
            }
    }

    unsafe fn map_range(
        &self,
        services: *mut uefi::BootServices,
        start: u64,
        end: u64,
        flags: u64,
    ) -> Result<(), uefi::Status> {
        if !start.is_multiple_of(PAGE_SIZE)
            || !end.is_multiple_of(PAGE_SIZE)
            || end < start
            || end > IDENTITY_LIMIT
        {
            return Err(uefi::LOAD_ERROR);
        }
        let mut address = start;
        while address < end {
            if address.is_multiple_of(TWO_MIB) && end - address >= TWO_MIB {
                self.map_2m(services, address, address, flags)?;
                address += TWO_MIB;
            } else {
                self.map_4k(services, address, address, flags)?;
                address += PAGE_SIZE;
            }
        }
        Ok(())
    }

    unsafe fn directory_entry(
        &self,
        services: *mut uefi::BootServices,
        address: u64,
    ) -> Result<*mut u64, uefi::Status> {
        let pml4 = child_table(services, self.root, ((address >> 39) & 511) as usize)?;
        let pdpt = child_table(services, pml4, ((address >> 30) & 511) as usize)?;
        Ok((pdpt as *mut u64).add(((address >> 21) & 511) as usize))
    }

    unsafe fn map_2m(
        &self,
        services: *mut uefi::BootServices,
        address: u64,
        physical: u64,
        flags: u64,
    ) -> Result<(), uefi::Status> {
        let entry = self.directory_entry(services, address)?;
        if *entry == 0 {
            *entry = physical | flags | HUGE;
        } else if *entry & HUGE == 0 {
            for page in 0..512 {
                self.map_4k(
                    services,
                    address + page * PAGE_SIZE,
                    physical + page * PAGE_SIZE,
                    flags,
                )?;
            }
        } else {
            *entry = physical | flags | HUGE;
        }
        Ok(())
    }

    unsafe fn map_4k(
        &self,
        services: *mut uefi::BootServices,
        address: u64,
        physical: u64,
        flags: u64,
    ) -> Result<(), uefi::Status> {
        let entry = self.directory_entry(services, address)?;
        if *entry & HUGE != 0 {
            let previous = *entry;
            let table = allocate_table(services)?;
            let base = previous & ADDRESS_MASK & !(TWO_MIB - 1);
            let inherited = previous & !ADDRESS_MASK & !HUGE;
            for index in 0..512 {
                *((table as *mut u64).add(index)) = (base + index as u64 * PAGE_SIZE) | inherited;
            }
            *entry = table | PRESENT | WRITABLE;
        }
        let table = if *entry == 0 {
            let table = allocate_table(services)?;
            *entry = table | PRESENT | WRITABLE;
            table
        } else {
            *entry & ADDRESS_MASK
        };
        *((table as *mut u64).add(((address >> 12) & 511) as usize)) = physical | flags;
        Ok(())
    }
}

unsafe fn allocate_table(services: *mut uefi::BootServices) -> Result<u64, uefi::Status> {
    let mut address = 0u64;
    let status = (*services).allocate_pages(uefi::EFI_LOADER_DATA, 1, &mut address);
    if status != uefi::SUCCESS {
        return Err(status);
    }
    if address >= IDENTITY_LIMIT {
        return Err(uefi::UNSUPPORTED);
    }
    ptr::write_bytes(address as *mut u8, 0, PAGE_SIZE as usize);
    Ok(address)
}

unsafe fn child_table(
    services: *mut uefi::BootServices,
    parent: u64,
    index: usize,
) -> Result<u64, uefi::Status> {
    let entry = (parent as *mut u64).add(index);
    if *entry == 0 {
        let child = allocate_table(services)?;
        *entry = child | PRESENT | WRITABLE;
        Ok(child)
    } else {
        Ok(*entry & ADDRESS_MASK)
    }
}

fn page_down(address: u64) -> u64 {
    address & !(PAGE_SIZE - 1)
}
fn page_up(address: u64) -> Option<u64> {
    address.checked_add(PAGE_SIZE - 1).map(page_down)
}

pub unsafe fn build(
    services: *mut uefi::BootServices,
    map: *const u8,
    map_size: usize,
    descriptor_size: usize,
    kernel: &KernelImage,
    info: &BootInfo,
    regions_bytes: usize,
) -> Result<PageTables, uefi::Status> {
    if info.framebuffer.overlaps(kernel.start, kernel.end)
        || info.framebuffer.overlaps(info.boot_info_physical, info.boot_info_physical + PAGE_SIZE)
        || info.framebuffer.overlaps(info.memory_regions, info.memory_regions + regions_bytes as u64)
    {
        return Err(uefi::LOAD_ERROR);
    }
    let tables = PageTables::new(services)?;
    let framebuffer = &info.framebuffer;
    let boot_info = info.boot_info_physical;
    let regions = info.memory_regions;
    for index in 0..map_size / descriptor_size {
        let descriptor =
            ptr::read_unaligned(map.add(index * descriptor_size) as *const uefi::MemoryDescriptor);
        if descriptor.pages == 0 {
            continue;
        }
        let bytes = descriptor
            .pages
            .checked_mul(PAGE_SIZE)
            .ok_or(uefi::LOAD_ERROR)?;
        let end = descriptor
            .physical_start
            .checked_add(bytes)
            .ok_or(uefi::LOAD_ERROR)?;
        // A four-level identity map cannot represent addresses in the upper canonical half.
        // High physical RAM remains in BootInfo and is reached through scratch-built aliases.
        if descriptor.physical_start >= IDENTITY_LIMIT {
            continue;
        }
        let end = end.min(IDENTITY_LIMIT);
        let executable = matches!(descriptor.kind, 1 | 3 | 5);
        // Firmware may place writable PE data in EfiLoaderCode pages. The
        // loader still runs briefly after CR3 changes; keep that allocation
        // writable until the kernel takes over and can discard it.
        let writable = descriptor.kind == uefi::EFI_LOADER_CODE || !executable;
        let uncached = matches!(descriptor.kind, 11 | 12);
        tables.map_range(
            services,
            descriptor.physical_start,
            end,
            tables.flags(writable, executable, uncached),
        )?;
    }

    if framebuffer.pixel_format != 0 && framebuffer.size != 0 {
        let end = framebuffer
            .base
            .checked_add(framebuffer.size)
            .ok_or(uefi::LOAD_ERROR)?;
        tables.map_range(
            services,
            page_down(framebuffer.base),
            page_up(end).ok_or(uefi::LOAD_ERROR)?,
            tables.flags(true, false, true),
        )?;
    }

    // Refine the loaded kernel from LoaderCode's broad executable mapping to
    // per-page permissions. GNU linkers may put two segments in the same page;
    // their permissions are combined for that page.
    let mut address = kernel.start;
    while address < kernel.end {
        let mut writable = false;
        let mut executable = false;
        for segment in &kernel.segments[..kernel.segment_count] {
            if address < segment.end && address + PAGE_SIZE > segment.start {
                writable |= segment.flags & 2 != 0;
                executable |= segment.flags & 1 != 0;
            }
        }
        if writable && executable {
            return Err(uefi::UNSUPPORTED);
        }
        tables.map_4k(
            services,
            address,
            address,
            tables.flags(writable, executable, false),
        )?;
        tables.map_4k(
            services,
            abi::KERNEL_IMAGE_BASE + (address - kernel.start),
            address,
            tables.flags(writable, executable, false),
        )?;
        address += PAGE_SIZE;
    }
    tables.map_4k(
        services,
        abi::BOOT_INFO_VA,
        boot_info,
        tables.flags(false, false, false),
    )?;
    for offset in (0..pages_for(regions_bytes)).map(|page| page as u64 * PAGE_SIZE) {
        tables.map_4k(
            services,
            abi::BOOT_REGIONS_VA + offset,
            regions + offset,
            tables.flags(false, false, false),
        )?;
    }
    if framebuffer.pixel_format != 0 {
        let size = page_up((framebuffer.base & (PAGE_SIZE - 1)) + framebuffer.size)
            .ok_or(uefi::LOAD_ERROR)?;
        for offset in (0..size).step_by(PAGE_SIZE as usize) {
            tables.map_4k(
                services,
                abi::MMIO_BASE + offset,
                page_down(framebuffer.base) + offset,
                tables.flags(true, false, true),
            )?;
        }
    }
    // A stable alias to the scratch leaf table lets the kernel access arbitrary
    // physical page-table frames before the permanent RAM aliases exist.
    const SCRATCH_TABLE: u64 = abi::BOOT_DATA_BASE + 0x20_0000;
    const SCRATCH: u64 = abi::BOOT_DATA_BASE + 0x40_0000;
    tables.map_4k(
        services,
        SCRATCH,
        boot_info,
        tables.flags(true, false, false),
    )?;
    let scratch_directory = tables.directory_entry(services, SCRATCH)?;
    let scratch_table = *scratch_directory & ADDRESS_MASK;
    tables.map_4k(
        services,
        SCRATCH_TABLE,
        scratch_table,
        tables.flags(true, false, false),
    )?;
    Ok(tables)
}

fn pages_for(bytes: usize) -> usize {
    bytes.div_ceil(PAGE_SIZE as usize)
}

pub unsafe fn enable_nx_and_switch(root: u64, nx_supported: bool) {
    if nx_supported {
        // Set EFER.NXE before loading PTEs with the NX bit.
        asm!(
            "mov ecx, 0xc0000080",
            "rdmsr",
            "bts eax, 11",
            "wrmsr",
            out("ecx") _, out("eax") _, out("edx") _,
            options(nostack),
        );
    }
    // Supervisor writes must respect read-only kernel mappings as well.
    let mut cr0: u64;
    asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
    cr0 |= 1 << 16; // CR0.WP
    asm!("mov cr0, {}", in(reg) cr0, options(nostack));
    asm!("mov cr3, {}", in(reg) root, options(nostack));
}
