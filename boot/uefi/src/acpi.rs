//! Copy selected firmware tables before ExitBootServices; never retain firmware pointers.
use crate::{
    abi::{
        acpi::{self, AcpiSnapshot},
        BootPayload,
    },
    payload::PayloadOwner,
    uefi,
};
use core::{mem::size_of, ptr, sync::atomic::AtomicBool};

#[repr(C)]
struct Configuration {
    guid: uefi::Guid,
    address: u64,
}
const ACPI2: uefi::Guid = uefi::Guid {
    data1: 0x8868e871,
    data2: 0xe4f1,
    data3: 0x11d3,
    data4: [0xbc, 0x22, 0, 0x80, 0xc7, 0x3c, 0x88, 0x81],
};
struct Map {
    bytes: *mut u8,
    length: usize,
    stride: usize,
    services: *mut uefi::BootServices,
}
impl Drop for Map {
    fn drop(&mut self) {
        unsafe {
            (*self.services).free_pool(self.bytes);
        }
    }
}
impl Map {
    unsafe fn read(&self, address: u64, length: usize) -> Result<&[u8], uefi::Status> {
        let end = address.checked_add(length as u64).ok_or(uefi::LOAD_ERROR)?;
        if address == 0 || end > 1u64 << 52 {
            return Err(uefi::LOAD_ERROR);
        }
        // Tables must fit wholly in a firmware-described RAM range. MMIO and unusable
        // ranges are excluded before constructing any slice from a firmware pointer.
        let mut covered = false;
        for offset in (0..self.length).step_by(self.stride) {
            let b = core::slice::from_raw_parts(self.bytes.add(offset), self.stride);
            let kind = acpi::u32_at(b, 0).map_err(|_| uefi::LOAD_ERROR)?;
            let base = acpi::u64_at(b, 8).map_err(|_| uefi::LOAD_ERROR)?;
            let pages = acpi::u64_at(b, 24).map_err(|_| uefi::LOAD_ERROR)?;
            let limit = pages
                .checked_mul(4096)
                .and_then(|n| base.checked_add(n))
                .ok_or(uefi::LOAD_ERROR)?;
            if matches!(kind, 1..=7 | 9 | 10) && base <= address && end <= limit {
                covered = true;
            }
        }
        if !covered {
            return Err(uefi::LOAD_ERROR);
        }
        Ok(core::slice::from_raw_parts(address as *const u8, length))
    }
    unsafe fn sdt(&self, address: u64) -> Result<&[u8], uefi::Status> {
        let h = self.read(address, 36)?;
        let length = acpi::u32_at(h, 4).map_err(|_| uefi::LOAD_ERROR)? as usize;
        if !(36..=acpi::MAX_BYTES).contains(&length) {
            return Err(uefi::LOAD_ERROR);
        }
        let b = self.read(address, length)?;
        acpi::sdt(b, b[..4].try_into().map_err(|_| uefi::LOAD_ERROR)?).map_err(|_| uefi::LOAD_ERROR)
    }
}
unsafe fn memory_map(services: *mut uefi::BootServices) -> Result<Map, uefi::Status> {
    // Bounded map storage avoids a retry/allocator cycle during discovery.
    let capacity = 4096 * 128;
    let mut bytes = ptr::null_mut();
    let status = (*services).allocate_pool(capacity, &mut bytes);
    if status != uefi::SUCCESS {
        return Err(status);
    }
    let mut map = Map {
        bytes,
        length: capacity,
        stride: 0,
        services,
    };
    let (mut key, mut version) = (0, 0);
    let status = (*services).get_memory_map(
        &mut map.length,
        map.bytes,
        &mut key,
        &mut map.stride,
        &mut version,
    );
    if status != uefi::SUCCESS {
        return Err(status);
    }
    if version != 1
        || map.stride < 40
        || map.length > capacity
        || !map.length.is_multiple_of(map.stride)
        || map.length / map.stride > 4096
    {
        return Err(uefi::LOAD_ERROR);
    }
    Ok(map)
}
/// The owner follows the existing payload ExitBootServices cleanup rule.
pub unsafe fn discover<'a>(
    table: *mut uefi::SystemTable,
    exit: &'a AtomicBool,
) -> Result<(AcpiSnapshot, PayloadOwner<'a>), uefi::Status> {
    let services = (*table).boot_services;
    let none = || {
        (
            AcpiSnapshot::NONE,
            PayloadOwner::new(services, BootPayload::NONE, exit),
        )
    };
    let count = (*table).number_of_table_entries;
    if count == 0 {
        return Ok(none());
    }
    if count > 1024 || (*table).configuration_table.is_null() {
        return Err(uefi::LOAD_ERROR);
    }
    let map = memory_map(services)?;
    let config = map.read(
        (*table).configuration_table as u64,
        count
            .checked_mul(size_of::<Configuration>())
            .ok_or(uefi::LOAD_ERROR)?,
    )?;
    let mut address = None;
    for chunk in config.as_chunks::<{ size_of::<Configuration>() }>().0 {
        let entry = ptr::read_unaligned(chunk.as_ptr() as *const Configuration);
        if entry.guid.data1 == ACPI2.data1
            && entry.guid.data2 == ACPI2.data2
            && entry.guid.data3 == ACPI2.data3
            && entry.guid.data4 == ACPI2.data4
            && address.replace(entry.address).is_some()
        {
            return Err(uefi::LOAD_ERROR);
        }
    }
    let Some(rsdp_address) = address else {
        return Ok(none());
    };
    let head = map.read(rsdp_address, 36)?;
    let length = acpi::u32_at(head, 20).map_err(|_| uefi::LOAD_ERROR)? as usize;
    if !(36..=4096).contains(&length) {
        return Err(uefi::LOAD_ERROR);
    }
    let rsdp = map.read(rsdp_address, length)?;
    let xsdt_address = acpi::rsdp(rsdp).map_err(|_| uefi::LOAD_ERROR)?;
    let xsdt = map.sdt(xsdt_address)?;
    let entries = acpi::xsdt(xsdt).map_err(|_| uefi::LOAD_ERROR)?;
    type SourceTable<'a> = Option<(&'a [u8; 8], u64, &'a [u8])>;
    let mut tables: [SourceTable<'_>; acpi::MAX_TABLES] = [None; acpi::MAX_TABLES];
    tables[0] = Some((b"RSD PTR ", rsdp_address, rsdp));
    tables[1] = Some((b"XSDT\0\0\0\0", xsdt_address, xsdt));
    let mut n = 2;
    for address in entries {
        let header = map.read(address, 36)?;
        let sig = match &header[..4] {
            b"APIC" => b"APIC\0\0\0\0",
            b"MCFG" => b"MCFG\0\0\0\0",
            b"HPET" => b"HPET\0\0\0\0",
            b"IVRS" => b"IVRS\0\0\0\0",
            _ => continue,
        };
        let bytes = map.sdt(address)?;
        if tables[..n].iter().flatten().any(|t| t.0 == sig) || n == acpi::MAX_TABLES {
            return Err(uefi::LOAD_ERROR);
        }
        tables[n] = Some((sig, address, bytes));
        n += 1;
    }
    let mut length = acpi::HEADER + n * acpi::RECORD;
    for (_, _, bytes) in tables[..n].iter().flatten() {
        length = (length + 7) & !7;
        length = length.checked_add(bytes.len()).ok_or(uefi::LOAD_ERROR)?;
    }
    if length > acpi::MAX_BYTES {
        return Err(uefi::LOAD_ERROR);
    }
    let mut physical = 0;
    let status =
        (*services).allocate_pages(uefi::EFI_LOADER_DATA, length.div_ceil(4096), &mut physical);
    if status != uefi::SUCCESS {
        return Err(status);
    }
    let owner = PayloadOwner::new(
        services,
        BootPayload::new(physical, length as u64).map_err(|_| uefi::LOAD_ERROR)?,
        exit,
    );
    let bytes = core::slice::from_raw_parts_mut(physical as *mut u8, length.div_ceil(4096) * 4096);
    bytes.fill(0);
    bytes[..8].copy_from_slice(b"FRACPI01");
    bytes[8..12].copy_from_slice(&(length as u32).to_le_bytes());
    bytes[12..16].copy_from_slice(&(n as u32).to_le_bytes());
    let mut offset = acpi::HEADER + n * acpi::RECORD;
    for (index, (sig, address, source)) in tables[..n].iter().flatten().enumerate() {
        offset = (offset + 7) & !7;
        let record = acpi::HEADER + index * acpi::RECORD;
        bytes[record..record + 8].copy_from_slice(*sig);
        bytes[record + 8..record + 16].copy_from_slice(&address.to_le_bytes());
        bytes[record + 16..record + 20].copy_from_slice(&(offset as u32).to_le_bytes());
        bytes[record + 20..record + 24].copy_from_slice(&(source.len() as u32).to_le_bytes());
        bytes[offset..offset + source.len()].copy_from_slice(source);
        offset += source.len();
    }
    acpi::Snapshot::parse(&bytes[..length]).map_err(|_| uefi::LOAD_ERROR)?;
    let descriptor = AcpiSnapshot {
        physical_start: physical,
        virtual_start: acpi::SNAPSHOT_VA,
        length: length as u64,
        reserved: 0,
    };
    descriptor.bounds().map_err(|_| uefi::LOAD_ERROR)?;
    Ok((descriptor, owner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::ManuallyDrop;
    #[test]
    fn firmware_slices_require_a_covering_ram_descriptor() {
        let source = [42u8; 64];
        let mut descriptor = [0u8; 40];
        descriptor[..4].copy_from_slice(&9u32.to_le_bytes());
        descriptor[8..16].copy_from_slice(&(source.as_ptr() as u64).to_le_bytes());
        descriptor[24..32].copy_from_slice(&1u64.to_le_bytes());
        let map = ManuallyDrop::new(Map {
            bytes: descriptor.as_mut_ptr(),
            length: 40,
            stride: 40,
            services: ptr::null_mut(),
        });
        assert_eq!(
            unsafe { map.read(source.as_ptr() as u64, 64) }.unwrap(),
            source
        );
        assert!(unsafe { map.read(source.as_ptr() as u64 - 1, 1) }.is_err());
        assert!(unsafe { map.read(u64::MAX, 64) }.is_err());
        // The same address in MMIO or unusable memory must never be dereferenced.
        let _ = map;
        for kind in [0u32, 8, 11, 12] {
            descriptor[..4].copy_from_slice(&kind.to_le_bytes());
            let map = ManuallyDrop::new(Map {
                bytes: descriptor.as_mut_ptr(),
                length: 40,
                stride: 40,
                services: ptr::null_mut(),
            });
            assert!(unsafe { map.read(source.as_ptr() as u64, 64) }.is_err());
        }
    }
    #[test]
    fn no_configuration_table_is_an_explicit_absent_snapshot() {
        let mut table: uefi::SystemTable = unsafe { core::mem::zeroed() };
        let exit = AtomicBool::new(false);
        let (snapshot, _owner) = unsafe { discover(&mut table, &exit) }.unwrap();
        assert_eq!(snapshot, AcpiSnapshot::NONE);
        table.number_of_table_entries = 1025;
        assert!(unsafe { discover(&mut table, &exit) }.is_err());
    }
}
