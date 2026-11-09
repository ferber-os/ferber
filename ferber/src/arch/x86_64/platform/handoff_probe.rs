//! Selected-controller diagnostics only; no domain or user device is published.
use super::{hpet::Clock, live_pci, mmio::MmioMapper, pci, xhci::Controller, Discovery};
use crate::boot_info::BootInfo;
fn overlap(a: u64, n: u64, b: u64, m: u64) -> bool {
    a < b.saturating_add(m) && b < a.saturating_add(n)
}
unsafe fn probe(info: &BootInfo) -> Result<(), &'static str> {
    let d = Discovery::from_boot_info(info).map_err(|_| "ACPI snapshot")?;
    let mcfg = d.mcfg().map_err(|_| "MCFG")?;
    let ivrs = d.ivrs().map_err(|_| "IVRS")?;
    let hpet = d.hpet().map_err(|_| "HPET")?;
    let r = &info.controller_resource;
    r.validate()
        .map_err(|_| "resource record")?
        .ok_or("missing controller resource")?;
    let mut mapper = MmioMapper::new(info).map_err(|_| "MMIO mapper")?;
    let inventory = live_pci::inventory(&mut mapper, &mcfg).map_err(|_| "PCI inventory")?;
    let (vendor, device) = if cfg!(feature = "xhci-target-msi") {
        (0x1022, 0x15b8)
    } else {
        (0x1b36, 0x000d)
    };
    let selected = inventory
        .select_xhci(live_pci::TargetProfile { vendor, device })
        .map_err(|_| "unique target")?;
    let f = selected.function;
    let raw = selected.read32(0x10).map_err(|_| "BAR")? as u64;
    let raw = raw
        | if raw & 6 == 4 {
            (selected.read32(0x14).map_err(|_| "BAR high")? as u64) << 32
        } else {
            0
        };
    if (f.segment, f.bus, f.device, f.function) != (r.segment, r.bus, r.slot, r.function)
        || (vendor, device) != (r.vendor, r.device)
        || selected.read32(0).map_err(|_| "identity")? != ((device as u32) << 16 | vendor as u32)
        || raw != r.raw_bar
        || selected.read32(8).map_err(|_| "class")? != r.class_revision
        || selected.read16(4).map_err(|_| "command")? & 2 == 0
    {
        return Err("stale resource identity/BAR");
    }
    let bytes = selected.snapshot().map_err(|_| "config snapshot")?;
    pci::Capabilities::parse(&bytes).map_err(|_| "PCI capabilities")?;
    pci::validate_extensions(|at| selected.read32(at)).map_err(|_| "PCI extensions")?;
    let (unit, alias) = ivrs
        .unit_for(f.segment, f.requester().map_err(|_| "requester")?)
        .map_err(|_| "requester coverage")?;
    for other in inventory.functions() {
        if other.function == f {
            continue;
        }
        let id = other
            .function
            .requester()
            .map_err(|_| "inventory requester")?;
        match ivrs.unit_for(other.function.segment, id) {
            Ok((u, a)) if u.segment == unit.segment && u.device == unit.device && a == alias => {
                return Err("shared requester alias")
            }
            Ok(_) => {}
            Err(_) => return Err("incomplete IVRS coverage"),
        }
    }
    // A bounded unique parent chain validates routing structure, not ACS/P2P isolation.
    let mut bus = f.bus;
    for depth in 0..=256 {
        let mut parent = None;
        for bridge in inventory.functions() {
            if bridge.function.segment != f.segment || bridge.identity.header != 1 {
                continue;
            }
            let buses = bridge.read32(0x18).map_err(|_| "bridge bus registers")?;
            let primary = buses as u8;
            let secondary = (buses >> 8) as u8;
            let subordinate = (buses >> 16) as u8;
            if secondary == 0 || secondary > subordinate || primary != bridge.function.bus {
                return Err("bridge routing");
            }
            if secondary == bus && parent.replace(primary).is_some() {
                return Err("ambiguous parent bridge");
            }
        }
        match parent {
            None => {
                if !mcfg
                    .regions()
                    .any(|r| r.segment == f.segment && r.first_bus == bus)
                {
                    return Err("missing parent bridge");
                }
                break;
            }
            Some(p) if p < bus && depth < 256 => bus = p,
            _ => return Err("bridge cycle"),
        }
    }
    for region in mcfg.regions() {
        if overlap(
            r.base,
            r.length,
            region.base + ((region.first_bus as u64) << 20),
            ((region.last_bus as u64 - region.first_bus as u64) + 1) << 20,
        ) {
            return Err("BAR overlaps ECAM");
        }
    }
    for u in ivrs.units() {
        if overlap(r.base, r.length, u.mmio, 0x4000) {
            return Err("BAR overlaps IOMMU");
        }
    }
    let madt = d.madt().map_err(|_| "MADT")?;
    let apic = madt.local_apic().map_err(|_| "local APIC")?;
    for (kind, data) in madt.entries() {
        if kind == 1 {
            let base = super::acpi::u32_at(data, 4).map_err(|_| "IOAPIC")? as u64;
            if overlap(r.base, r.length, base, 0x1000) {
                return Err("BAR overlaps IOAPIC");
            }
        }
    }
    if overlap(r.base, r.length, hpet.address, 0x1000) || overlap(r.base, r.length, apic, 0x1000) {
        return Err("BAR overlaps platform control");
    }
    let clock = Clock::initialize(
        mapper
            .map(hpet.address, 0x1000)
            .map_err(|_| "HPET mapping")?,
    )
    .map_err(|_| "HPET clock")?;
    let registers = mapper.map(r.base, r.length).map_err(|_| "BAR mapping")?;
    let controller = Controller::new(selected, registers).map_err(|_| "xHCI register layout")?;
    crate::kprintln!(
        "[xhci] selected {:04x}:{:02x}:{:02x}.{} BAR {:#x}+{:#x} IOMMU {:#x} alias {:#x}",
        f.segment,
        f.bus,
        f.device,
        f.function,
        r.base,
        r.length,
        unit.mmio,
        alias
    );
    controller
        .diagnostic_handoff(
            &clock,
            raw,
            ((device as u32) << 16) | vendor as u32,
            r.class_revision,
        )
        .map_err(|e| {
            crate::kprintln!("[xhci] failed stage: {:?}", e);
            "controller handoff"
        })?;
    Ok(())
}
pub unsafe fn run(info: &BootInfo) -> ! {
    match probe(info) {
        Ok(()) => crate::kprintln!("[FERBER_TEST] xhci_handoff: PASS"),
        Err(stage) => crate::kprintln!("[FERBER_TEST] xhci_handoff: FAIL ({})", stage),
    }
    crate::kprintln!("[xhci] diagnostic stopped; physical DMA isolation is not established");
    loop {
        core::arch::asm!("cli; hlt", options(nomem, nostack));
    }
}
