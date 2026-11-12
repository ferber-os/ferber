//! Read-only physical takeover observations. This path never activates an IOMMU.
use super::{live_pci, mmio::MmioMapper, pci, Discovery};
use crate::boot_info::BootInfo;
use alloc::vec::Vec;
#[path = "../../../../../shared/pci_readiness.rs"]
mod observations;
#[derive(Clone, Copy)]
struct Snapshot {
    function: observations::Function,
    command: u16,
    coverage: Option<(u16, u16)>,
    extensions: observations::ExtensionSnapshot,
}
fn state(value: Result<Option<bool>, observations::Error>) -> &'static str {
    match value {
        Err(_) => "UNKNOWN",
        Ok(None) => "-",
        Ok(Some(false)) => "off",
        Ok(Some(true)) => "ON",
    }
}
fn print_unknown(function: observations::Function, extensions: &observations::ExtensionSnapshot) {
    if let Err(error) = extensions.result {
        let f = function;
        match extensions.last_read {
                Some((offset, Ok(value))) => crate::kprintln!(
                    "[ready] UNKNOWN {:04x}:{:02x}:{:02x}.{} {:?} offset={:#x} value={:#x} raw100={:x?}",
                    f.segment, f.requester >> 8, (f.requester >> 3) & 31, f.requester & 7,
                    error, offset, value, extensions.raw100
                ),
                Some((offset, Err(_))) => crate::kprintln!(
                    "[ready] UNKNOWN {:04x}:{:02x}:{:02x}.{} {:?} offset={:#x} value=UNREADABLE raw100={:x?}",
                    f.segment, f.requester >> 8, (f.requester >> 3) & 31, f.requester & 7,
                    error, offset, extensions.raw100
                ),
                None => crate::kprintln!(
                    "[ready] UNKNOWN {:04x}:{:02x}:{:02x}.{} {:?} no-read-context",
                    f.segment, f.requester >> 8, (f.requester >> 3) & 31, f.requester & 7, error
                ),
            }
    }
}
fn overlaps(a: u64, n: u64, b: u64, m: u64) -> bool {
    a < b.saturating_add(m) && b < a.saturating_add(n)
}
unsafe fn probe(info: &BootInfo) -> Result<(), &'static str> {
    let d = Discovery::from_boot_info(info).map_err(|_| "ACPI snapshot")?;
    let mcfg = d.mcfg().map_err(|_| "MCFG")?;
    let ivrs = d.ivrs().map_err(|_| "IVRS")?;
    let resource = &info.controller_resource;
    resource
        .validate()
        .map_err(|_| "resource record")?
        .ok_or("missing controller resource")?;
    let mut mapper = MmioMapper::new(info).map_err(|_| "MMIO mapper")?;
    let inventory = live_pci::inventory(&mut mapper, &mcfg).map_err(|_| "PCI inventory")?;
    // shortcut: this boot report accepts 256 functions; raise after bounding larger-fabric reporting costs.
    if inventory.functions().len() > 256 {
        return Err("inventory report budget");
    }
    let (vendor, device) = if cfg!(feature = "xhci-target-msi") {
        (0x1022, 0x15b8)
    } else {
        (0x1b36, 0x000d)
    };
    let selected = inventory
        .select_xhci(live_pci::TargetProfile { vendor, device })
        .map_err(|_| "unique target")?;
    let f = selected.function;
    let selected_rid = f.requester().map_err(|_| "selected requester")?;
    let raw = selected.read32(0x10).map_err(|_| "selected BAR")? as u64;
    let raw = raw
        | if raw & 6 == 4 {
            (selected.read32(0x14).map_err(|_| "selected BAR high")? as u64) << 32
        } else {
            0
        };
    if (f.segment, f.bus, f.device, f.function)
        != (
            resource.segment,
            resource.bus,
            resource.slot,
            resource.function,
        )
        || (vendor, device) != (resource.vendor, resource.device)
        || selected
            .read32(0)
            .map_err(|_| "selected identity readback")?
            != ((device as u32) << 16 | vendor as u32)
        || raw != resource.raw_bar
        || selected.read32(8).map_err(|_| "selected class")? != resource.class_revision
    {
        return Err("stale selected resource");
    }
    let (selected_unit, selected_alias) = ivrs
        .unit_for(f.segment, selected_rid)
        .map_err(|_| "selected IVRS coverage")?;
    let mut snapshots = Vec::new();
    let mut functions = Vec::new();
    snapshots
        .try_reserve_exact(inventory.functions().len())
        .map_err(|_| "snapshot quota")?;
    functions
        .try_reserve_exact(inventory.functions().len())
        .map_err(|_| "routing quota")?;
    let mut uncovered = 0;
    let mut bus_mastering = 0;
    let mut extensions_summary = observations::ExtensionSummary::default();
    for function in inventory.functions() {
        let f = function.function;
        let rid = f.requester().map_err(|_| "requester")?;
        let coverage = ivrs
            .unit_for_optional(f.segment, rid)
            .map_err(|_| "ambiguous IVRS coverage")?
            .map(|(unit, alias)| (unit.device, alias));
        let command = function.read16(4).map_err(|_| "PCI command")?;
        if command & 4 != 0 {
            bus_mastering += 1;
        }
        let extensions = observations::capture_extensions(|at| {
            function.read32(at).map_err(|_| observations::Error::Read)
        });
        extensions_summary.record(extensions.result);
        let mut node = observations::Function {
            segment: f.segment,
            requester: rid,
            header: function.identity.header,
            primary: 0,
            secondary: 0,
            subordinate: 0,
        };
        print_unknown(node, &extensions);
        if function.identity.header == 1 {
            let buses = function
                .read32(0x18)
                .map_err(|_| "bridge routing snapshot")?;
            node.primary = buses as u8;
            node.secondary = (buses >> 8) as u8;
            node.subordinate = (buses >> 16) as u8;
        }
        let root = mcfg
            .regions()
            .any(|r| r.segment == f.segment && r.first_bus == f.bus);
        if coverage.is_none() {
            // Structural exceptions describe this report only; they never authorize activation.
            pci::diagnostic_unit_for(&ivrs, f, function.identity, root)
                .map_err(|_| "uncovered ordinary requester")?;
            uncovered += 1;
        }
        functions.push(node);
        snapshots.push(Snapshot {
            function: node,
            command,
            coverage,
            extensions,
        });
    }
    let mut invalid_paths = 0;
    let mut shared_aliases = 0;
    let mut selected_path = None;
    for (index, snapshot) in snapshots.iter().enumerate() {
        let f = snapshot.function;
        let mut roots = [0u8; 256];
        let mut count = 0;
        for region in mcfg.regions().filter(|r| r.segment == f.segment) {
            if count == roots.len() {
                return Err("root bus report budget");
            }
            roots[count] = region.first_bus;
            count += 1;
        }
        let path = observations::ancestry(&functions, f.segment, f.requester, &roots[..count]);
        if path.is_err() {
            invalid_paths += 1;
        }
        if f.segment == selected.function.segment && f.requester == selected_rid {
            selected_path = Some(path.as_ref().map(|p| p.count as isize).unwrap_or(-1));
        }
        let shared = snapshot.coverage.is_some_and(|coverage| {
            snapshots[..index].iter().any(|other| {
                other.function.segment == f.segment && other.coverage == Some(coverage)
            })
        });
        if shared {
            shared_aliases += 1;
        }
        let (unit, alias) = snapshot.coverage.unwrap_or((0xffff, 0xffff));
        crate::kprintln!("[ready] {:04x}:{:04x} unit {:04x} alias {:04x} BME {} INTx-mask {} ATS {} PRI {} PASID {} parents {}{}",
            f.segment,f.requester,unit,alias,u8::from(snapshot.command&4!=0),u8::from(snapshot.command&0x400!=0),
            state(snapshot.extensions.result.map(|e|e.ats)),state(snapshot.extensions.result.map(|e|e.pri)),state(snapshot.extensions.result.map(|e|e.pasid)),
            path.as_ref().map(|p|p.count).unwrap_or(usize::MAX),if shared {" SHARED"} else {""});
        if let Ok(Some(acs)) = snapshot.extensions.result.map(|e| e.acs) {
            crate::kprintln!(
                "[ready] ACS {:04x}:{:04x} capability {:#x} control {:#x}",
                f.segment,
                f.requester,
                acs.capability,
                acs.control
            );
        }
        if let Ok(path) = path {
            for bridge in &path.bridges[..path.count] {
                crate::kprintln!("[ready]   via {:04x}:{:04x}", f.segment, bridge);
            }
        }
    }
    let mut active_units = 0;
    let mut missing_smi = 0;
    let mut missing_invalidate = 0;
    let mut locked_smi_allow = 0;
    for unit in ivrs.units() {
        for region in mcfg.regions() {
            if overlaps(
                unit.mmio,
                0x4000,
                region.base + ((region.first_bus as u64) << 20),
                ((region.last_bus as u64 - region.first_bus as u64) + 1) << 20,
            ) {
                return Err("IOMMU overlaps ECAM");
            }
        }
        if overlaps(unit.mmio, 0x4000, resource.base, resource.length) {
            return Err("IOMMU overlaps controller BAR");
        }
        let registers = mapper
            .map(unit.mmio, 0x4000)
            .map_err(|_| "IOMMU register snapshot mapping")?;
        let control = registers
            .read64(0x18)
            .map_err(|_| "IOMMU control snapshot")?;
        let status = registers
            .read64(0x2020)
            .map_err(|_| "IOMMU status snapshot")?;
        let features = registers
            .read64(0x30)
            .map_err(|_| "IOMMU feature snapshot")?;
        if control & 1 != 0 || status & 0x18 != 0 {
            active_units += 1;
        }
        if features & (1 << 6) == 0 {
            missing_invalidate += 1;
        }
        if (features >> 16) & 3 != 1 || (features >> 18) & 7 > 4 {
            missing_smi += 1;
        } else {
            for index in 0..(1usize << ((features >> 18) & 7)) {
                let filter = registers
                    .read64(0x60 + index * 8)
                    .map_err(|_| "SMI filter snapshot")?;
                if filter & (3 << 16) == 3 << 16 {
                    locked_smi_allow += 1;
                }
            }
        }
        crate::kprintln!(
            "[ready] unit {:04x}:{:04x} control {:#x} status {:#x} features {:#x}",
            unit.segment,
            unit.device,
            control,
            status,
            features
        );
    }
    // Keep the footer together instead of straddling the early console's page clear.
    super::super::console::begin_readiness_summary();
    crate::kprintln!(
        "[ready] selected {:04x}:{:02x}:{:02x}.{} unit {:04x} alias {:04x} parents {}",
        f.segment,
        f.bus,
        f.device,
        f.function,
        selected_unit.device,
        selected_alias,
        selected_path.ok_or("missing selected routing snapshot")?
    );
    crate::kprintln!(
        "[ready] functions {} structural-uncovered {} BME {} known-extensions-on {} unknown-extensions {}",
        snapshots.len(),
        uncovered,
        bus_mastering,
        extensions_summary.known_enabled,
        extensions_summary.unknown
    );
    crate::kprintln!("[ready] invalid-paths {} shared-aliases {} active-units {} missing-IASup {} missing-SMI-filter {}",invalid_paths,shared_aliases,active_units,missing_invalidate,missing_smi);
    crate::kprintln!(
        "[ready] firmware-locked valid SMI filters {}",
        locked_smi_allow
    );
    for snapshot in &snapshots {
        print_unknown(snapshot.function, &snapshot.extensions);
    }
    if invalid_paths != 0 {
        return Err("invalid or unsupported bridge ancestry");
    }
    if !extensions_summary.complete() {
        return Err("PCI extension snapshot");
    }
    crate::kprintln!("[FERBER_TEST] iommu_inventory: PASS");
    Ok(())
}
pub unsafe fn run(info: &BootInfo) -> ! {
    if let Err(stage) = probe(info) {
        crate::kprintln!("[FERBER_TEST] iommu_inventory: FAIL ({})", stage);
    }
    crate::kprintln!(
        "[ready] BLOCKED: requester halt/reset and interrupt containment not established"
    );
    crate::kprintln!("[ready] BLOCKED: firmware DMA / posted-write drain not established");
    crate::kprintln!("[ready] BLOCKED: complete ACS/P2P isolation and exclusive firmware ownership not established");
    crate::kprintln!("[FERBER_TEST] iommu_readiness: BLOCKED");
    crate::kprintln!("[ready] hardware registers observed only; no assignment or IOMMU activation");
    loop {
        core::arch::asm!("cli; hlt", options(nomem, nostack));
    }
}
