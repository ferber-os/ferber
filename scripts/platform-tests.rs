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
    assert!(acpi::SNAPSHOT_VA >= boot_payload_window::PAYLOAD_BASE + boot_payload_window::PAYLOAD_WINDOW_BYTES);
}
