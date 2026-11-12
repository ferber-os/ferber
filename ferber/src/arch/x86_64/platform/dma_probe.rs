//! Opt-in QEMU EDU DMA experiment. Never authorizes physical device assignment.
use super::{
    hpet::Clock,
    iommu::{
        encoding::{self, Engine},
        Coherent,
    },
    live_pci::{self, ConfigFunction},
    mmio::{MappedRegion, MmioMapper},
    pci, Discovery,
};
use crate::{boot_info::BootInfo, kernel::memory::vmo::Vmo};
use alloc::vec::Vec;
use core::mem::ManuallyDrop;
#[path = "../../../../../shared/amdvi_eligibility.rs"]
mod eligibility;
const _: () = assert!(
    (cfg!(feature = "qemu-dma-unmapped") as u8
        + cfg!(feature = "qemu-dma-readonly") as u8
        + cfg!(feature = "qemu-dma-unmap") as u8
        + cfg!(feature = "qemu-dma-production") as u8)
        <= 1,
    "select one QEMU DMA scenario"
);
const DOMAIN: u16 = 1;
const IOVA: usize = 0x0100_0000;
const COUNT: usize = 64;
fn scenario() -> &'static str {
    if cfg!(feature = "qemu-dma-production") {
        "production"
    } else if cfg!(feature = "qemu-dma-unmapped") {
        "unmapped"
    } else if cfg!(feature = "qemu-dma-readonly") {
        "readonly"
    } else if cfg!(feature = "qemu-dma-unmap") {
        "unmap"
    } else {
        "allowed"
    }
}
struct Endpoint(ConfigFunction);
impl Endpoint {
    fn finish(self) -> Result<(), &'static str> {
        unsafe { self.0.mask_bus_mastering() }.map_err(|_| "final EDU containment")?;
        core::mem::forget(self);
        Ok(())
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        // All exits attempt containment; DMA backing below remains retained regardless.
        if unsafe { self.0.mask_bus_mastering() }.is_err() {
            crate::kprintln!("[dma] containment failed; backing retained until reboot");
        }
    }
}
unsafe fn command(f: ConfigFunction, enabled: bool) -> Result<(), &'static str> {
    let old = f.read16(4).map_err(|_| "EDU command")?;
    f.write16(4, if enabled { old | 4 } else { old & !4 })
        .map_err(|_| "EDU command")?;
    if f.read16(4).map_err(|_| "EDU readback")? & 4 != if enabled { 4 } else { 0 } {
        return Err("EDU BME readback");
    }
    Ok(())
}
unsafe fn transfer(
    r: MappedRegion,
    clock: &Clock,
    source: u64,
    destination: u64,
    write: bool,
) -> Result<(), &'static str> {
    if r.read64(0x98).map_err(|_| "EDU DMA state")? & 1 != 0 {
        return Err("EDU DMA busy");
    }
    r.write64(0x80, source).map_err(|_| "EDU source")?;
    r.write64(0x88, destination)
        .map_err(|_| "EDU destination")?;
    r.write64(0x90, COUNT as u64).map_err(|_| "EDU count")?;
    r.write64(0x98, if write { 3 } else { 1 })
        .map_err(|_| "EDU start")?;
    clock
        .wait_until(1_000_000, || {
            Ok(r.read64(0x98).map_err(|_| super::acpi::Error)? & 1 == 0)
        })
        .map_err(|_| "EDU DMA deadline")
}
unsafe fn quiesce_sata(
    f: ConfigFunction,
    mapper: &mut MmioMapper,
    clock: &Clock,
) -> Result<(), &'static str> {
    // Trusted fixed QEMU AHCI model: no disks other than the read-only boot CD.
    let raw = f.read32(0x24).map_err(|_| "AHCI BAR")?;
    let bar = pci::MemoryBar::decode(raw, 0, 4096).map_err(|_| "AHCI BAR")?;
    let r = mapper
        .map(bar.base, bar.length)
        .map_err(|_| "AHCI mapping")?;
    let ports = r.read32(0x0c).map_err(|_| "AHCI ports")?;
    r.write32(4, r.read32(4).map_err(|_| "AHCI GHC")? & !2)
        .map_err(|_| "AHCI interrupts")?;
    for p in 0..32 {
        if ports & (1 << p) == 0 {
            continue;
        }
        let at = 0x100 + p * 0x80;
        if r.read32(at + 0x38).map_err(|_| "AHCI CI")? != 0
            || r.read32(at + 0x34).map_err(|_| "AHCI SACT")? != 0
        {
            return Err("AHCI active DMA");
        }
        r.write32(at + 0x14, 0).map_err(|_| "AHCI port interrupt")?;
        let c = r.read32(at + 0x18).map_err(|_| "AHCI command")?;
        r.write32(at + 0x18, c & !1).map_err(|_| "AHCI stop")?;
        clock
            .wait_until(1_000_000, || {
                Ok(r.read32(at + 0x18).map_err(|_| super::acpi::Error)? & (1 << 15) == 0)
            })
            .map_err(|_| "AHCI CR deadline")?;
        let c = r.read32(at + 0x18).map_err(|_| "AHCI command")?;
        r.write32(at + 0x18, c & !(1 << 4))
            .map_err(|_| "AHCI FIS stop")?;
        clock
            .wait_until(1_000_000, || {
                Ok(r.read32(at + 0x18).map_err(|_| super::acpi::Error)? & (1 << 14) == 0)
            })
            .map_err(|_| "AHCI FR deadline")?;
    }
    Ok(())
}
unsafe fn denied(
    engine: &mut Engine<Coherent>,
    r: MappedRegion,
    clock: &Clock,
    f: ConfigFunction,
    backing: &Vmo,
    initial: &[u8; 3 * 4096],
    guard: usize,
) -> Result<(), &'static str> {
    crate::kprintln!("[dma] expected fault address {:#x}", guard + 64);
    command(f, true)?;
    transfer(r, clock, 0x40000, (guard + 64) as u64, true)?;
    command(f, false)?;
    let mut deadline = clock.deadline(1_000_000).map_err(|_| "fault deadline")?;
    let fault = loop {
        if let Some(fault) = engine.qemu_dma_fault().map_err(|_| "fault record")? {
            break fault;
        }
        if clock.expired(&mut deadline).map_err(|_| "fault clock")? {
            return Err("missing DMA fault");
        }
    };
    crate::kprintln!(
        "[dma] fault requester {:#x} address {:#x} permissions {}",
        fault.requester,
        fault.address,
        fault.permissions
    );
    if fault.requester != 0x20 || fault.address != (guard + 64) as u64 || fault.permissions != 2 {
        return Err("mismatched DMA fault");
    }
    let mut actual = [0u8; 3 * 4096];
    backing
        .read_owned(0, &mut actual)
        .map_err(|_| "sentinel verification")?;
    if actual != *initial || !engine.poisoned() {
        return Err("denied DMA changed memory");
    }
    Ok(())
}
unsafe fn probe(info: &BootInfo) -> Result<&'static str, &'static str> {
    let d = Discovery::from_boot_info(info).map_err(|_| "ACPI snapshot")?;
    let mcfg = d.mcfg().map_err(|_| "MCFG")?;
    let ivrs = d.ivrs().map_err(|_| "IVRS")?;
    if ivrs.units().count() != 1 {
        return Err("single IOMMU required");
    }
    let unit = ivrs.units().next().ok_or("IOMMU")?;
    let mut mapper = MmioMapper::new(info).map_err(|_| "MMIO mapper")?;
    let inventory = live_pci::inventory(&mut mapper, &mcfg).map_err(|_| "PCI inventory")?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(inventory.functions().len())
        .map_err(|_| "inventory quota")?;
    for f in inventory.functions() {
        let rid = f.function.requester().map_err(|_| "requester")?;
        let coverage = ivrs
            .unit_for_optional(f.function.segment, rid)
            .map_err(|_| "ambiguous IVRS")?
            .map(|(u, alias)| eligibility::Coverage {
                unit: u.device,
                alias,
            });
        let i = f.identity;
        entries.push(eligibility::Entry {
            segment: f.function.segment,
            requester: rid,
            vendor: i.vendor,
            device: i.device,
            class: i.class,
            subclass: i.subclass,
            interface: i.interface,
            header: i.header,
            coverage,
            extensions_valid: pci::validate_extensions(|at| f.read32(at)).is_ok(),
        });
        crate::kprintln!(
            "[dma] inventory {:04x}:{:04x} {:04x}:{:04x} class {:02x}:{:02x}:{:02x}",
            f.function.segment,
            rid,
            i.vendor,
            i.device,
            i.class,
            i.subclass,
            i.interface
        );
    }
    if let Err(error) = eligibility::qemu_edu(&entries) {
        crate::kprintln!("[dma] eligibility rejection: {:?}", error);
        // Only this exact extra test endpoint makes the negative topology case pass.
        if entries.len() == 8
            && entries.iter().any(|e| {
                e.segment == 0 && e.requester == 0x28 && e.vendor == 0x1234 && e.device == 0x11e8
            })
        {
            let filtered: Vec<_> = entries
                .iter()
                .filter(|e| e.requester != 0x28)
                .copied()
                .collect();
            eligibility::qemu_edu(&filtered).map_err(|_| "unexpected topology baseline")?;
            return Ok("topology");
        }
        return Err("topology eligibility");
    }
    if unit.segment != 0 || unit.device != 8 {
        return Err("IOMMU identity");
    }
    let hpet = d.hpet().map_err(|_| "HPET")?;
    let clock = Clock::initialize(mapper.map(hpet.address, 4096).map_err(|_| "HPET map")?)
        .map_err(|_| "HPET clock")?;
    let f = *inventory
        .functions()
        .iter()
        .find(|f| f.function.requester() == Ok(0x20))
        .ok_or("EDU missing")?;
    let endpoint = Endpoint(f);
    f.mask_bus_mastering().map_err(|_| "EDU containment")?;
    let raw = f.read32(0x10).map_err(|_| "EDU BAR")?;
    if raw & 6 != 0 || f.read16(4).map_err(|_| "EDU command")? & 2 == 0 {
        return Err("EDU BAR format/decoding");
    }
    let bar = pci::MemoryBar::decode(raw, 0, 1 << 20).map_err(|_| "EDU BAR")?;
    let r = mapper.map(bar.base, bar.length).map_err(|_| "EDU MMIO")?;
    if r.read32(0).map_err(|_| "EDU identification")? & 0xffff != 0x00ed
        || r.read64(0x98).map_err(|_| "EDU idle")? & 1 != 0
    {
        return Err("EDU identity/idle");
    }
    for other in inventory.functions() {
        if other.identity.device == 0x2922 {
            quiesce_sata(*other, &mut mapper, &clock)?;
        }
        if other.function.requester() == Ok(8) {
            let bytes = other.snapshot().map_err(|_| "IOMMU PCI snapshot")?;
            let msi = eligibility::qemu_iommu_msi_offset(&bytes)
                .map_err(|_| "IOMMU model capabilities")?;
            other.clear_bus_mastering().map_err(|_| "IOMMU BME")?;
            other
                .write16(4, other.read16(4).map_err(|_| "IOMMU command")? | 0x400)
                .map_err(|_| "IOMMU INTx")?;
            other
                .write16(
                    msi + 2,
                    other.read16(msi + 2).map_err(|_| "IOMMU MSI")? & !1,
                )
                .map_err(|_| "IOMMU MSI mask")?;
            if other.read16(4).map_err(|_| "IOMMU command")? & 0x404 != 0x400
                || other.read16(msi + 2).map_err(|_| "IOMMU MSI")? & 1 != 0
            {
                return Err("IOMMU interrupt readback");
            }
        } else {
            other
                .mask_bus_mastering()
                .map_err(|_| "requester containment")?;
        }
    }
    // This is a trusted fixed emulator contract, not evidence of physical posted-write draining.
    let registers = mapper.map(unit.mmio, 0x4000).map_err(|_| "IOMMU map")?;
    let backing = ManuallyDrop::new(Vmo::create_contiguous(3 * 4096).map_err(|_| "DMA backing")?);
    let mut initial = [0xa5u8; 3 * 4096];
    for i in 0..COUNT {
        initial[64 + i] = (i as u8) ^ 0x73;
    }
    backing
        .write_owned(0, &initial)
        .map_err(|_| "DMA initialization")?;
    let coherent = Coherent::new(registers, clock).map_err(|_| "IOMMU allocation")?;
    let mut engine = Engine::new(coherent);
    if cfg!(feature = "qemu-dma-production") {
        let before = registers
            .read64(encoding::CONTROL)
            .map_err(|_| "IOMMU control")?;
        let features = registers
            .read64(encoding::EXTENDED_FEATURES)
            .map_err(|_| "IOMMU features")?;
        if features & encoding::IASUP == 0 || (features >> 16) & 3 != 0 {
            return Err("production fixture features");
        }
        if engine.enable() != Err(encoding::Error::Invalid)
            || engine.enabled()
            || registers
                .read64(encoding::CONTROL)
                .map_err(|_| "IOMMU control")?
                != before
        {
            return Err("production rejection");
        }
        crate::kprintln!("[dma] production rejected missing SMI filtering before activation");
        endpoint.finish()?;
        return Ok("production");
    }
    engine.enable_qemu_dma_probe().map_err(|e| {
        crate::kprintln!("[dma] enable: {:?}", e);
        "IOMMU activation"
    })?;
    let frames = [
        backing.frames().get(0).ok_or("DMA frame")? as u64,
        backing.frames().get(1).ok_or("DMA frame")? as u64,
        backing.frames().get(2).ok_or("DMA frame")? as u64,
    ];
    engine
        .map(IOVA, &frames, 3, DOMAIN)
        .map_err(|_| "DMA mapping")?;
    engine
        .assign(0x20, DOMAIN, 0xf0, 0)
        .map_err(|_| "EDU assignment")?;
    command(f, true)?;
    transfer(r, &clock, (IOVA + 64) as u64, 0x40000, false)?;
    transfer(r, &clock, 0x40000, (IOVA + 4096 + 64) as u64, true)?;
    command(f, false)?;
    engine.check_faults().map_err(|_| "roundtrip fault")?;
    let mut actual = [0u8; 3 * 4096];
    backing
        .read_owned(0, &mut actual)
        .map_err(|_| "DMA verification")?;
    initial.copy_within(64..64 + COUNT, 4096 + 64);
    if actual != initial {
        for (index, (got, want)) in actual
            .iter()
            .zip(initial.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(8)
        {
            crate::kprintln!(
                "[dma] mismatch offset {:#x}: got {:#x}, expected {:#x}",
                index,
                got,
                want
            );
        }
        crate::kprintln!(
            "[dma] EDU DMA source {:x?} destination {:x?} count {:x?} command {:x?}",
            r.read64(0x80),
            r.read64(0x88),
            r.read64(0x90),
            r.read64(0x98)
        );
        return Err("DMA roundtrip/guards");
    }
    if scenario() == "allowed" {
        engine.deny(0x20, DOMAIN).map_err(|_| "EDU deny")?;
        endpoint.finish()?;
        return Ok("allowed");
    }
    // An untranslated write would hit this retained sentinel, never arbitrary memory.
    let guard = frames[2] as usize;
    if guard == IOVA || guard >= encoding::IOVA_LIMIT {
        return Err("sentinel IOVA");
    }
    if scenario() != "unmapped" {
        engine
            .map(
                guard,
                &frames[2..],
                if scenario() == "readonly" { 1 } else { 3 },
                DOMAIN,
            )
            .map_err(|_| "sentinel mapping")?;
        if scenario() == "unmap" {
            // Populate the hardware IOTLB before testing invalidation on unmap.
            command(f, true)?;
            transfer(r, &clock, 0x40000, (guard + 64) as u64, true)?;
            command(f, false)?;
            engine.check_faults().map_err(|_| "priming DMA fault")?;
            initial.copy_within(64..64 + COUNT, 8192 + 64);
            backing
                .read_owned(0, &mut actual)
                .map_err(|_| "priming DMA verification")?;
            if actual != initial {
                return Err("priming DMA/guards");
            }
            engine
                .unmap(guard, 4096, DOMAIN)
                .map_err(|_| "sentinel unmap")?;
        }
    }
    denied(&mut engine, r, &clock, f, &backing, &initial, guard)?;
    endpoint.finish()?;
    Ok(scenario())
}
pub unsafe fn run(info: &BootInfo) -> ! {
    match probe(info) {
        Ok(name) => crate::kprintln!("[FERBER_TEST] qemu_dma_{}: PASS", name),
        Err(stage) => crate::kprintln!("[FERBER_TEST] qemu_dma_{}: FAIL ({})", scenario(), stage),
    }
    crate::kprintln!("[dma] QEMU DMA isolation only; no physical, SMI or IRQ isolation claim");
    loop {
        core::arch::asm!("cli; hlt", options(nomem, nostack));
    }
}
