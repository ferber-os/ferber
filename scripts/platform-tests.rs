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
        pub fn length(&self) -> usize { self.length }
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
#[test]
fn hpet_initialization_identifies_rejected_capabilities_before_writes() {
    let valid = (100_000_000u64 << 32) | (1 << 13) | 1;
    for (cap, error) in [
        ((1 << 13) | 1, hpet::InitError::Period),
        (((100_000_001u64) << 32) | (1 << 13) | 1, hpet::InitError::Period),
        (valid & !(1 << 13), hpet::InitError::CounterWidth),
        (valid & !255, hpet::InitError::Revision),
    ] {
        let mut words = vec![0x1234u64; 512];
        words[0] = cap;
        let before = words.clone();
        let result = unsafe { hpet::Clock::initialize(mmio::MappedRegion {
            base: words.as_mut_ptr() as usize, length: 4096, physical: 0xfed00000,
        }) };
        assert_eq!(result.err(), Some(error));
        assert_eq!(words, before);
    }
}
#[test]
fn hpet_initialization_reports_bounded_register_failures() {
    for (length, timers, error) in [
        (0, 0, hpet::InitError::Capabilities),
        (16, 0, hpet::InitError::Disable),
        (0x100, 0, hpet::InitError::TimerRead(0)),
        (0x108, 1, hpet::InitError::TimerRead(1)),
    ] {
        let mut words = vec![0u64; 512];
        words[0] = (100_000_000u64 << 32) | (1 << 13) | 1 | (timers << 8);
        let result = unsafe { hpet::Clock::initialize(mmio::MappedRegion {
            base: words.as_mut_ptr() as usize, length, physical: 0xfed00000,
        }) };
        assert_eq!(result.err(), Some(error));
    }
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

#[test]
fn pci_translation_extensions_require_disabled_controls() {
    for (id, enable) in [(0x0f, 1u32 << 31), (0x13, 1), (0x1b, 1 << 16)] {
        let mut config = [0u32; 1024];
        config[0x100 / 4] = id | (1 << 16);
        assert!(pci::validate_extensions(|at| Ok(config[at / 4])).is_ok());
        config[0x104 / 4] = enable;
        assert!(pci::validate_extensions(|at| Ok(config[at / 4])).is_err());
        // Capability/feature bits must not be mistaken for control enables.
        config[0x104 / 4] = if id == 0x1b { 0x1f06 } else if id == 0x0f { 31 } else { 0 };
        assert!(pci::validate_extensions(|at| Ok(config[at / 4])).is_ok());
    }
}
#[test]
fn pci_extended_chains_reject_cycles_invalid_links_and_truncation() {
    for absent in [0, u32::MAX] {
        assert!(pci::validate_extensions(|_| Ok(absent)).is_ok());
    }
    for next in [0x080, 0x101, 0x100, 0xfff] {
        assert!(pci::validate_extensions(|at| if at == 0x100 { Ok(1 | (1 << 16) | (next << 20)) } else { Err(acpi::Error) }).is_err());
    }
    let mut config = [0u32; 1024];
    config[0x100 / 4] = 1 | (1 << 16) | (0x200 << 20);
    config[0x200 / 4] = 2 | (1 << 16) | (0x100 << 20);
    assert!(pci::validate_extensions(|at| Ok(config[at / 4])).is_err());
    for bad in [0, u32::MAX, 1] {
        config[0x200 / 4] = bad;
        assert!(pci::validate_extensions(|at| Ok(config[at / 4])).is_err());
    }
    for id in [0x0f, 0x13, 0x1b] {
        config[0x200 / 4] = id | (1 << 16);
        assert!(pci::validate_extensions(|at| if at >= 0x204 { Err(acpi::Error) } else { Ok(config[at / 4]) }).is_err());
        config[0x200 / 4] = 1 | (1 << 16) | (0xffc << 20);
        config[0xffc / 4] = id | (1 << 16);
        assert!(pci::validate_extensions(|at| Ok(config[at / 4])).is_err());
    }
}

#[test]
fn pci_capability_payload_overlap_is_rejected() {
    let mut b = [0u8; 256];
    b[..2].copy_from_slice(&0x1022u16.to_le_bytes());
    b[6] = 0x10;
    b[0x34] = 0x40;
    for (kind, control, next) in [(5, 0x80u16, 0x44), (5, 0x100, 0x50), (0x10, 0, 0x48)] {
        b[0x40..].fill(0);
        b[0x40] = kind;
        b[0x41] = next;
        b[0x42..0x44].copy_from_slice(&control.to_le_bytes());
        b[next as usize] = 0x11;
        assert!(pci::Capabilities::parse(&b).is_err());
    }
    for id in [0x0f, 0x13, 0x1b] {
        let mut config = [0u32; 1024];
        config[0x100 / 4] = id | (1 << 16) | (0x104 << 20);
        config[0x104 / 4] = 2 | (2 << 16);
        assert!(pci::validate_extensions(|at| Ok(config[at / 4])).is_err());
    }
}

#[test]
fn hpet_accepts_unspecified_width_but_not_arbitrary_widths() {
    for width in [0, 64, 32, 1, 255] {
        let mut b = sdt(b"HPET", 56);
        b[41] = width;
        b[44..52].copy_from_slice(&0xfed00000u64.to_le_bytes());
        sum(&mut b);
        assert_eq!(tables::Hpet::parse(&b).is_ok(), matches!(width, 0 | 64));
    }
}

// Captured MSI B650M-A PRO WIFI IVRS: root bridge 0000 and IOMMU 0002 are outside requester scopes.
fn physical_msi_ivrs() -> [u8; 200] {
    [
        0x49, 0x56, 0x52, 0x53, 0xc8, 0x00, 0x00, 0x00, 0x02, 0x2d, 0x41, 0x4d, 0x44, 0x20, 0x20, 0x00,
        0x41, 0x6d, 0x64, 0x54, 0x61, 0x62, 0x6c, 0x65, 0x01, 0x00, 0x00, 0x00, 0x41, 0x4d, 0x44, 0x20,
        0x01, 0x00, 0x00, 0x00, 0x41, 0x30, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x10, 0xb0, 0x44, 0x00, 0x02, 0x00, 0x40, 0x00, 0x00, 0x00, 0x70, 0xf7, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x6e, 0x8f, 0x04, 0x80, 0x03, 0x03, 0x00, 0x00, 0x04, 0xfe, 0xff, 0x00,
        0x43, 0x00, 0xff, 0x00, 0x00, 0xa5, 0x00, 0x00, 0x04, 0xff, 0xff, 0x00, 0x48, 0x00, 0x00, 0x00,
        0x00, 0xa0, 0x00, 0x02, 0x48, 0x00, 0x00, 0xd7, 0x20, 0xa0, 0x00, 0x01, 0x48, 0x00, 0x00, 0x00,
        0x21, 0x01, 0x00, 0x01, 0x11, 0x30, 0x54, 0x00, 0x02, 0x00, 0x40, 0x00, 0x00, 0x00, 0x70, 0xf7,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x04, 0x00, 0xfa, 0x4a, 0x25, 0xa2,
        0xef, 0x77, 0x65, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x03, 0x00, 0x00,
        0x04, 0xfe, 0xff, 0x00, 0x43, 0x00, 0xff, 0x00, 0x00, 0xa5, 0x00, 0x00, 0x04, 0xff, 0xff, 0x00,
        0x48, 0x00, 0x00, 0x00, 0x00, 0xa0, 0x00, 0x02, 0x48, 0x00, 0x00, 0xd7, 0x20, 0xa0, 0x00, 0x01,
        0x48, 0x00, 0x00, 0x00, 0x21, 0x01, 0x00, 0x01,
    ]
}

fn coverage_function(requester: u16) -> pci::Function {
    pci::Function { segment: 0, bus: (requester >> 8) as u8,
        device: ((requester >> 3) & 31) as u8, function: (requester & 7) as u8 }
}
fn coverage_identity(class: u8, subclass: u8) -> pci::Identity {
    pci::Identity { vendor: 0x1022, device: 0x14d8, class, subclass,
        interface: 0, header: 0, multifunction: false }
}
#[test]
fn physical_ivrs_distinguishes_structural_functions_from_requesters() {
    let bytes = physical_msi_ivrs();
    let parsed = tables::Ivrs::parse(&bytes).unwrap();
    assert_eq!(parsed.units().count(), 1);
    assert_eq!(parsed.unit_for(0, 0x1200).unwrap().1, 0x1200);
    for id in [0, 2] {
        assert!(parsed.unit_for_optional(0, id).unwrap().is_none());
        assert!(parsed.unit_for(0, id).is_err());
    }
    let bridge = coverage_identity(6, 0);
    let iommu = coverage_identity(8, 6);
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(0), bridge, true).unwrap().is_none());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(2), iommu, true).unwrap().is_none());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(0x1200), coverage_identity(0x0c, 3), false).unwrap().is_some());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(0), bridge, false).is_err());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(0), coverage_identity(0x0c, 3), true).is_err());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(1), iommu, true).is_err());
    for (id, mut identity) in [(0, bridge), (2, iommu)] {
        identity.header = 1;
        assert!(pci::diagnostic_unit_for(&parsed, coverage_function(id), identity, true).is_err());
        identity.header = 0;
        identity.interface = 1;
        assert!(pci::diagnostic_unit_for(&parsed, coverage_function(id), identity, true).is_err());
    }
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(2), coverage_identity(8, 5), true).is_err());
    let mut wrong_segment = coverage_function(2);
    wrong_segment.segment = 1;
    assert!(pci::diagnostic_unit_for(&parsed, wrong_segment, iommu, true).is_err());
}
#[test]
fn diagnostic_coverage_never_exempts_ambiguity_or_nonroot_hosts() {
    let only_target = ivrs(&[ivhd(0x11, &[2, 0, 0x12, 0])]);
    let parsed = tables::Ivrs::parse(&only_target).unwrap();
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(0x100), coverage_identity(6, 0), false).is_err());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(0x100), coverage_identity(8, 6), false).is_err());
    let a = ivhd(0x11, &[1, 0, 0, 0]);
    let mut b = a.clone();
    b[4..6].copy_from_slice(&3u16.to_le_bytes());
    b[8..16].copy_from_slice(&0xf7704000u64.to_le_bytes());
    let ambiguous = ivrs(&[a, b]);
    let parsed = tables::Ivrs::parse(&ambiguous).unwrap();
    assert!(parsed.unit_for_optional(0, 0).is_err());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(0), coverage_identity(6, 0), true).is_err());
    assert!(pci::diagnostic_unit_for(&parsed, coverage_function(2), coverage_identity(8, 6), true).is_err());
}
