#[path = "../shared/acpi.rs"]
mod acpi;
#[path = "../ferber/src/arch/x86_64/platform/pci.rs"]
mod pci;
#[path = "../shared/acpi_tables.rs"]
mod tables;
fn sum(b: &mut [u8]) {
    b[9] = 0;
    b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |a, b| a.wrapping_add(*b)));
}
fn sdt(sig: &[u8; 4], len: usize) -> Vec<u8> {
    let mut b = vec![0; len];
    b[..4].copy_from_slice(sig);
    b[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    b[8] = 1;
    sum(&mut b);
    b
}
fn ivhd(kind: u8, entries: &[u8]) -> Vec<u8> {
    let n = if kind == 0x10 { 24 } else { 40 };
    let mut b = vec![0; n + entries.len()];
    b[0] = kind;
    let len = b.len() as u16;
    b[2..4].copy_from_slice(&len.to_le_bytes());
    b[4..6].copy_from_slice(&2u16.to_le_bytes());
    b[6..8].copy_from_slice(&0x40u16.to_le_bytes());
    b[8..16].copy_from_slice(&0xf7700000u64.to_le_bytes());
    b[n..].copy_from_slice(entries);
    b
}
fn ivrs(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut b = sdt(b"IVRS", 48);
    for p in parts {
        b.extend_from_slice(p);
    }
    let len = b.len() as u32;
    b[4..8].copy_from_slice(&len.to_le_bytes());
    sum(&mut b);
    b
}
#[test]
fn ivhd_deduplicates_and_rejects_conflicting_scope() {
    let b = ivrs(&[ivhd(0x10, &[1, 0, 0, 0]), ivhd(0x11, &[1, 0, 0, 0])]);
    let parsed = tables::Ivrs::parse(&b).unwrap();
    assert_eq!(parsed.units().count(), 1);
    assert_eq!(parsed.unit_for(0, 0x1200).unwrap().0.kind, 0x11);
    let bad = ivrs(&[ivhd(0x10, &[1, 0, 0, 0]), ivhd(0x11, &[2, 0, 0x12, 0])]);
    assert!(tables::Ivrs::parse(&bad).is_err());
}
#[test]
fn ivhd_alias_and_ranges_are_bounded() {
    let entries = [0x43, 0, 0x12, 0, 0, 0, 0x13, 0, 4, 7, 0x12, 0];
    let b = ivrs(&[ivhd(0x11, &entries)]);
    let parsed = tables::Ivrs::parse(&b).unwrap();
    assert_eq!(parsed.unit_for(0, 0x1203).unwrap().1, 0x1300);
    assert!(parsed.unit_for(0, 0x1208).is_err());
    assert!(tables::Ivrs::parse(&ivrs(&[ivhd(0x11, &entries[..8])])).is_err());
}
#[test]
fn ivmd_is_fail_closed_and_truncations_never_panic() {
    let good = ivrs(&[ivhd(0x11, &[1, 0, 0, 0])]);
    for n in 0..good.len() {
        assert!(tables::Ivrs::parse(&good[..n]).is_err());
    }
    let mut m = vec![0u8; 32];
    m[0] = 0x20;
    m[2] = 32;
    assert!(tables::Ivrs::parse(&ivrs(&[ivhd(0x11, &[1, 0, 0, 0]), m])).is_err());
}
#[test]
fn ecam_checks_ranges_and_uses_absolute_bus_number() {
    let mut b = sdt(b"MCFG", 60);
    b[44..52].copy_from_slice(&0xe0000000u64.to_le_bytes());
    b[54] = 16;
    b[55] = 31;
    sum(&mut b);
    let m = tables::Mcfg::parse(&b).unwrap();
    let e = m.regions().next().unwrap();
    assert_eq!(e.address(16, 2, 1, 0x100).unwrap(), 0xe1011100);
    assert!(e.address(0, 0, 0, 0).is_err());
    assert!(e.address(16, 32, 0, 0).is_err());
    b[55] = 15;
    sum(&mut b);
    assert!(tables::Mcfg::parse(&b).is_err());
}
#[test]
fn madt_and_hpet_validate_entry_lengths_and_addresses() {
    let mut b = sdt(b"APIC", 46);
    b[36..40].copy_from_slice(&0xfee00000u32.to_le_bytes());
    b[44] = 0;
    b[45] = 2;
    sum(&mut b);
    assert!(tables::Madt::parse(&b).is_err());
    let mut h = sdt(b"HPET", 56);
    h[41] = 64;
    h[44..52].copy_from_slice(&0xfed00000u64.to_le_bytes());
    sum(&mut h);
    assert_eq!(tables::Hpet::parse(&h).unwrap().address, 0xfed00000);
    h[40] = 1;
    sum(&mut h);
    assert!(tables::Hpet::parse(&h).is_err());
}
#[test]
fn pci_identity_capabilities_and_bar_do_not_touch_hardware() {
    let mut b = [0u8; 256];
    b[..2].copy_from_slice(&0x1022u16.to_le_bytes());
    b[2..4].copy_from_slice(&0x15b8u16.to_le_bytes());
    b[9..12].copy_from_slice(&[0x30, 3, 12]);
    assert!(pci::Identity::parse(&b).unwrap().unwrap().is_xhci());
    b[6] = 0x10;
    b[0x34] = 0x40;
    b[0x40] = 5;
    assert_eq!(pci::Capabilities::parse(&b).unwrap().entries().count(), 1);
    b[0x41] = 0x40;
    assert!(pci::Capabilities::parse(&b).is_err());
    assert!(pci::MemoryBar::decode(0xfed00004, 0, 0x10000).is_ok());
    assert!(pci::MemoryBar::decode(1, 0, 4096).is_err());
    assert_eq!(
        pci::Function {
            segment: 0,
            bus: 0x12,
            device: 0,
            function: 0
        }
        .requester()
        .unwrap(),
        0x1200
    );
}
#[test]
fn snapshot_descriptor_rejects_partial_absence_and_overflow() {
    assert_eq!(acpi::AcpiSnapshot::NONE.bounds(), Ok(None));
    let mut d = acpi::AcpiSnapshot {
        physical_start: 0x1000,
        virtual_start: acpi::SNAPSHOT_VA,
        length: 4096,
        reserved: 0,
    };
    assert_eq!(d.bounds(), Ok(Some((0x1000, 0x2000))));
    d.reserved = 1;
    assert!(d.bounds().is_err());
    d.reserved = 0;
    d.physical_start = u64::MAX & !4095;
    assert!(d.bounds().is_err());
}
fn snapshot() -> Vec<u8> {
    let mut r = vec![0; 36];
    r[..8].copy_from_slice(b"RSD PTR ");
    r[15] = 2;
    r[20..24].copy_from_slice(&36u32.to_le_bytes());
    r[24..32].copy_from_slice(&0x2000u64.to_le_bytes());
    r[8] = 0u8.wrapping_sub(r[..20].iter().fold(0u8, |a, b| a.wrapping_add(*b)));
    r[32] = 0u8.wrapping_sub(r.iter().fold(0u8, |a, b| a.wrapping_add(*b)));
    let x = sdt(b"XSDT", 36);
    let tables = [(b"RSD PTR ", 0x1000u64, r), (b"XSDT\0\0\0\0", 0x2000, x)];
    let mut b = vec![0; 80];
    b[..8].copy_from_slice(b"FRACPI01");
    b[12..16].copy_from_slice(&2u32.to_le_bytes());
    for (i, (sig, address, data)) in tables.iter().enumerate() {
        while b.len() & 7 != 0 {
            b.push(0);
        }
        let offset = b.len() as u32;
        let p = 16 + i * 32;
        b[p..p + 8].copy_from_slice(*sig);
        b[p + 8..p + 16].copy_from_slice(&address.to_le_bytes());
        b[p + 16..p + 20].copy_from_slice(&offset.to_le_bytes());
        b[p + 20..p + 24].copy_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
    }
    let len = b.len() as u32;
    b[8..12].copy_from_slice(&len.to_le_bytes());
    b
}
#[test]
fn owned_snapshot_validates_complete_structure_and_checksums() {
    let b = snapshot();
    let parsed = acpi::Snapshot::parse(&b).unwrap();
    assert_eq!(parsed.tables().count(), 2);
    assert_eq!(parsed.table(b"XSDT\0\0\0\0").unwrap().physical, 0x2000);
    for n in 0..b.len() {
        assert!(acpi::Snapshot::parse(&b[..n]).is_err());
    }
    let mut bad = b.clone();
    *bad.last_mut().unwrap() ^= 1;
    assert!(acpi::Snapshot::parse(&bad).is_err());
    let mut bad = b.clone();
    bad[16 + 24] = 1;
    assert!(acpi::Snapshot::parse(&bad).is_err());
    let mut bad = b;
    bad[16 + 32 + 8] = 1;
    assert!(acpi::Snapshot::parse(&bad).is_err());
}

#[path = "../shared/boot_payload.rs"]
mod boot_payload_window;
#[test]
fn acpi_and_payload_alias_windows_do_not_overlap() {
    assert!(
        acpi::SNAPSHOT_VA
            >= boot_payload_window::PAYLOAD_BASE + boot_payload_window::PAYLOAD_WINDOW_BYTES
    );
}
#[path = "../shared/platform_mmio.rs"]
mod mmio_bounds;
#[test]
fn register_apertures_reject_all_ram_and_rounding_overlap() {
    let protected = [(0x1000, 0x3000), (0x100000, 0x200000)];
    assert_eq!(
        mmio_bounds::validate(0xfed00100, 0x100, 48, &protected),
        Ok((0xfed00000, 0xfed01000))
    );
    for (p, n) in [
        (0, 4096),
        (0x300, 4096),
        (0x1fff, 1),
        (0x1fffff, 4096),
        (u64::MAX, 4096),
        (1u64 << 48, 4096),
    ] {
        assert!(mmio_bounds::validate(p, n, 48, &protected).is_err());
    }
}
#[test]
fn bar_sizing_masks_are_width_checked() {
    assert_eq!(pci::bar_length(0xffff0004, u32::MAX, true).unwrap(), 65536);
    assert_eq!(pci::bar_length(0xffff0000, 0, false).unwrap(), 65536);
    assert!(pci::bar_length(0xffff0f00, 0, false).is_err());
    assert!(pci::bar_length(0xffff0001, 0, false).is_err());
}

// Synthetic ECAM backing: never accesses host PCI configuration space.
mod mmio {
    #[derive(Clone, Copy, Debug)]
    pub struct MappedRegion {
        pub base: usize,
        pub length: usize,
        pub physical: u64,
    }
    impl MappedRegion {
        pub fn virtual_base(&self) -> usize {
            self.base
        }
        pub fn subregion(&self, offset: usize, length: usize) -> Result<Self, ()> {
            if offset.checked_add(length).is_none_or(|e| e > self.length) {
                return Err(());
            }
            Ok(Self {
                base: self.base + offset,
                length,
                physical: self.physical + offset as u64,
            })
        }
        pub fn read64(&self, at: usize) -> Result<u64, ()> {
            if at & 7 != 0 || at + 8 > self.length {
                return Err(());
            }
            Ok(unsafe { core::ptr::read_volatile((self.base + at) as *const u64) })
        }
        pub unsafe fn write64(&self, at: usize, value: u64) -> Result<(), ()> {
            if at & 7 != 0 || at + 8 > self.length {
                return Err(());
            }
            core::ptr::write_volatile((self.base + at) as *mut u64, value);
            Ok(())
        }
        pub fn read32(&self, at: usize) -> Result<u32, ()> {
            if at & 3 != 0 || at + 4 > self.length {
                return Err(());
            }
            Ok(unsafe { core::ptr::read_volatile((self.base + at) as *const u32) })
        }
        pub unsafe fn write32(&self, at: usize, value: u32) -> Result<(), ()> {
            if at & 3 != 0 || at + 4 > self.length {
                return Err(());
            }
            core::ptr::write_volatile((self.base + at) as *mut u32, value);
            Ok(())
        }
    }
    pub struct MmioMapper {
        pub region: MappedRegion,
    }
    impl MmioMapper {
        pub unsafe fn map(&mut self, physical: u64, length: u64) -> Result<MappedRegion, ()> {
            if physical != self.region.physical || length != self.region.length as u64 {
                return Err(());
            }
            Ok(self.region)
        }
    }
}
extern crate alloc;
#[path = "../ferber/src/arch/x86_64/platform/live_pci.rs"]
mod live_pci;
#[test]
fn synthetic_pci_inventory_unique_selection_and_containment() {
    let mut backing = vec![u32::MAX; (1 << 20) / 4];
    let bytes =
        unsafe { core::slice::from_raw_parts_mut(backing.as_mut_ptr() as *mut u8, 1 << 20) };
    let mut config = [0u8; 256];
    config[..2].copy_from_slice(&0x1022u16.to_le_bytes());
    config[2..4].copy_from_slice(&0x15b8u16.to_le_bytes());
    config[4..6].copy_from_slice(&0x407u16.to_le_bytes());
    config[6..8].copy_from_slice(&0x8010u16.to_le_bytes());
    config[9..12].copy_from_slice(&[0x30, 3, 12]);
    config[0x34] = 0x40;
    config[0x40] = 5;
    config[0x42] = 1;
    bytes[..256].copy_from_slice(&config);
    let mut b = sdt(b"MCFG", 60);
    b[44..52].copy_from_slice(&0xe0000000u64.to_le_bytes());
    sum(&mut b);
    let m = tables::Mcfg::parse(&b).unwrap();
    let mut mapper = mmio::MmioMapper {
        region: mmio::MappedRegion {
            base: bytes.as_mut_ptr() as usize,
            length: 1 << 20,
            physical: 0xe0000000,
        },
    };
    let inv = unsafe { live_pci::inventory(&mut mapper, &m) }.unwrap();
    assert_eq!(inv.functions().len(), 1);
    let profile = live_pci::TargetProfile {
        vendor: 0x1022,
        device: 0x15b8,
    };
    let target = inv.select_xhci(profile).unwrap();
    unsafe { target.mask_bus_mastering() }.unwrap();
    assert_eq!(target.read16(4).unwrap() & 4, 0);
    assert_eq!(target.read16(6).unwrap(), 0x8010);
    assert_eq!(target.read16(0x42).unwrap() & 1, 0);
    // A corrupt optional capability list must still leave mandatory DMA/INTx
    // containment in place, without clearing unrelated W1C status bits.
    bytes[4] |= 4;
    bytes[0x41] = 0x40;
    assert!(unsafe { target.mask_bus_mastering() }.is_err());
    assert_eq!(target.read16(4).unwrap() & 0x404, 0x400);
    assert_eq!(target.read16(6).unwrap(), 0x8010);
    bytes[0x41] = 0;
    bytes[0x8000..0x8100].copy_from_slice(&config);
    let inv = unsafe { live_pci::inventory(&mut mapper, &m) }.unwrap();
    assert_eq!(inv.functions().len(), 2);
    assert!(inv.select_xhci(profile).is_err());
}

#[path = "../ferber/src/arch/x86_64/platform/hpet.rs"]
mod hpet;
#[test]
fn hardware_deadlines_use_specified_hpet_period() {
    assert_eq!(hpet::ticks_for_us(100_000_000, 100_000).unwrap(), 1_000_000);
    assert_eq!(hpet::ticks_for_us(41_666_667, 1).unwrap(), 24);
    assert!(hpet::ticks_for_us(0, 1).is_err());
    assert!(hpet::ticks_for_us(100_000_001, 1).is_err());
    assert!(hpet::ticks_for_us(1, u64::MAX).is_err());
}
#[path = "../ferber/src/arch/x86_64/platform/xhci.rs"]
mod xhci;
#[test]
fn xhci_handoff_preserves_reserved_bits_and_disables_all_smi_enables() {
    let mut ecam = vec![u32::MAX; (1 << 20) / 4];
    let bytes = unsafe { core::slice::from_raw_parts_mut(ecam.as_mut_ptr() as *mut u8, 1 << 20) };
    bytes[..256].fill(0);
    bytes[..2].copy_from_slice(&0x1022u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&0x15b8u16.to_le_bytes());
    bytes[9..12].copy_from_slice(&[0x30, 3, 12]);
    let mut b = sdt(b"MCFG", 60);
    b[44..52].copy_from_slice(&0xe0000000u64.to_le_bytes());
    sum(&mut b);
    let m = tables::Mcfg::parse(&b).unwrap();
    let mut mapper = mmio::MmioMapper {
        region: mmio::MappedRegion {
            base: bytes.as_mut_ptr() as usize,
            length: 1 << 20,
            physical: 0xe0000000,
        },
    };
    let inv = unsafe { live_pci::inventory(&mut mapper, &m) }.unwrap();
    let pci = inv.functions()[0];
    let mut hp = vec![0u64; 512];
    hp[0] = (100_000_000u64 << 32) | (1 << 13) | 1;
    let clock = unsafe {
        hpet::Clock::initialize(mmio::MappedRegion {
            base: hp.as_mut_ptr() as usize,
            length: 4096,
            physical: 0xfed00000,
        })
    }
    .unwrap();
    assert!(unsafe { pci.function_level_reset(&clock) }.is_err());
    let mut registers = vec![0u32; 1024];
    registers[0] = (0x100 << 16) | 0x40;
    registers[4] = 0x20 << 16;
    registers[0x80 / 4] = 1;
    registers[0x84 / 4] = 0xe1fee | 0xe011;
    let controller = unsafe {
        xhci::Controller::new(
            pci,
            mmio::MappedRegion {
                base: registers.as_mut_ptr() as usize,
                length: 4096,
                physical: 0xf0000000,
            },
        )
    }
    .unwrap();
    unsafe { controller.firmware_handoff(&clock) }.unwrap();
    assert_ne!(registers[0x80 / 4] & (1 << 24), 0);
    assert_eq!(registers[0x84 / 4], 0xe1fee);
}
