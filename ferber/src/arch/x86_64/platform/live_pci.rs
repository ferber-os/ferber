//! Kernel-only PCI inventory and containment. No bus mastering is enabled here.
#![allow(dead_code)]
use super::{
    acpi::Error,
    mmio::{MappedRegion, MmioMapper},
    pci::{Capabilities, Function, Identity, MemoryBar},
    tables::Mcfg,
};
use alloc::vec::Vec;
#[derive(Clone, Copy, Debug)]
pub struct ConfigFunction {
    pub function: Function,
    pub identity: Identity,
    registers: MappedRegion,
}
impl ConfigFunction {
    pub fn read32(&self, offset: usize) -> Result<u32, Error> {
        self.registers.read32(offset).map_err(|_| Error)
    }
    pub fn read16(&self, offset: usize) -> Result<u16, Error> {
        if offset & 1 != 0 || offset > 4094 {
            return Err(Error);
        }
        let d = self.read32(offset & !3)?;
        Ok((d >> ((offset & 2) * 8)) as u16)
    }
    /// # Safety
    /// Kernel owns this configuration register and its hardware protocol.
    pub unsafe fn write32(&self, offset: usize, value: u32) -> Result<(), Error> {
        self.registers.write32(offset, value).map_err(|_| Error)
    }
    pub(super) unsafe fn write16(&self, offset: usize, value: u16) -> Result<(), Error> {
        if offset & 1 != 0 || offset > 4094 {
            return Err(Error);
        }
        core::ptr::write_volatile((self.registers.virtual_base() + offset) as *mut u16, value);
        Ok(())
    }
    pub fn snapshot(&self) -> Result<[u8; 256], Error> {
        let mut b = [0; 256];
        for i in (0..256).step_by(4) {
            b[i..i + 4].copy_from_slice(&self.read32(i)?.to_le_bytes());
        }
        Ok(b)
    }
    /// Mask legacy/MSI/MSI-X requests and clear BME. This prevents new requests;
    /// it does NOT prove completion of in-flight DMA or controller quiescence.
    /// # Safety
    /// Exclusive kernel ownership, IF=0, no concurrent configuration accesses.
    pub unsafe fn mask_bus_mastering(&self) -> Result<(), Error> {
        // Contain new DMA and legacy IRQs before inspecting optional capability
        // data. Malformed chains must never leave bus mastering enabled.
        self.write16(4, (self.read16(4)? & !4) | 0x400)?;
        if self.read16(4)? & 0x404 != 0x400 {
            return Err(Error);
        }
        if self.identity.header == 2 {
            return Err(Error);
        }
        let bytes = self.snapshot()?;
        for cap in Capabilities::parse(&bytes)?.entries() {
            let at = cap.offset as usize;
            let control = self.read16(at + 2)?;
            match cap.id {
                5 => self.write16(at + 2, control & !1)?,
                0x11 => self.write16(at + 2, (control | 0x4000) & !0x8000)?,
                _ => {}
            }
        }
        Ok(())
    }
    /// Reset only an advertised PCIe FLR function, never a whole bridge subtree.
    /// Transactions Pending must clear before reset; no best-effort continuation.
    /// # Safety
    /// Exclusive kernel function ownership. Firmware handoff is complete. An
    /// actual HPET clock runs, and no user mappings or IRQ routes are published.
    pub unsafe fn function_level_reset(&self, clock: &super::hpet::Clock) -> Result<(), Error> {
        if self.identity.header != 0 {
            return Err(Error);
        }
        let bytes = self.snapshot()?;
        let cap = Capabilities::parse(&bytes)?
            .entries()
            .find(|c| c.id == 0x10)
            .ok_or(Error)?
            .offset as usize;
        if cap + 0x14 > 256 || self.read32(cap + 4)? & (1 << 28) == 0 {
            return Err(Error);
        }
        self.mask_bus_mastering()?;
        clock.wait_until(1_000_000, || Ok(self.read16(cap + 0x0a)? & (1 << 5) == 0))?;
        self.write16(cap + 8, self.read16(cap + 8)? | (1 << 15))?;
        // PCIe6.6.2: no configuration requests for at least100ms after FLR.
        clock.wait_us(100_000)?;
        clock.wait_until(1_000_000, || Ok(self.read16(0)? == self.identity.vendor))?;
        if self.read16(4)? & 4 != 0 || self.read16(cap + 0x0a)? & (1 << 5) != 0 {
            return Err(Error);
        }
        self.mask_bus_mastering()?;
        Ok(())
    }

    /// Size one assigned memory BAR with decoding off, then restore its exact
    /// address and command. Only called after controller-level quiescence.
    /// # Safety
    /// The device is reset/halted with no in-flight DMA, and configuration is
    /// exclusively kernel owned. Sizing must precede user MMIO delegation.
    pub unsafe fn size_memory_bar(&self, index: u8) -> Result<MemoryBar, Error> {
        if self.identity.header != 0 || index >= 6 {
            return Err(Error);
        }
        let at = 0x10 + index as usize * 4;
        let low = self.read32(at)?;
        let wide = low & 6 == 4;
        if low & 1 != 0 || !matches!((low >> 1) & 3, 0 | 2) || wide && index == 5 {
            return Err(Error);
        }
        let high = if wide { self.read32(at + 4)? } else { 0 };
        let command = self.read16(4)?;
        if command & 4 != 0 {
            return Err(Error);
        }
        self.write16(4, command & !3)?;
        self.write32(at, u32::MAX)?;
        if wide {
            self.write32(at + 4, u32::MAX)?;
        }
        let mask_low = self.read32(at)?;
        let mask_high = if wide { self.read32(at + 4)? } else { u32::MAX };
        // Reads above cannot escape a checked configuration page. Restore before
        // validating the mask so malformed device responses leave no changed BAR.
        self.write32(at, low)?;
        if wide {
            self.write32(at + 4, high)?;
        }
        self.write16(4, command)?;
        let length = super::pci::bar_length(mask_low, mask_high, wide)?;
        MemoryBar::decode(low, high, length)
    }
}
pub struct Inventory {
    functions: Vec<ConfigFunction>,
}
#[derive(Clone, Copy)]
pub struct TargetProfile {
    pub vendor: u16,
    pub device: u16,
}
impl Inventory {
    pub fn functions(&self) -> &[ConfigFunction] {
        &self.functions
    }
    /// Identity selection must be unique. Board BDFs never become a generic ABI.
    pub fn select_xhci(&self, profile: TargetProfile) -> Result<ConfigFunction, Error> {
        let mut found = None;
        for f in &self.functions {
            if f.identity.is_xhci()
                && f.identity.vendor == profile.vendor
                && f.identity.device == profile.device
                && found.replace(*f).is_some()
            {
                return Err(Error);
            }
        }
        found.ok_or(Error)
    }
    /// # Safety
    /// Boot CPU owns the complete PCI fabric after firmware exit. This only
    /// masks new traffic; isolation activation additionally requires draining
    /// or resetting every affected DMA requester, including bridge aliases.
    pub unsafe fn mask_all_bus_mastering(&self) -> Result<(), Error> {
        for f in &self.functions {
            f.mask_bus_mastering()?;
        }
        Ok(())
    }
}
/// Discover only. Register writes and interrupts are not enabled by enumeration.
/// # Safety
/// Boot-only exclusive mapper. MCFG comes from the validated retained snapshot.
pub unsafe fn inventory(mapper: &mut MmioMapper, mcfg: &Mcfg<'_>) -> Result<Inventory, Error> {
    let mut functions = Vec::new();
    for e in mcfg.regions() {
        let start = e.base + ((e.first_bus as u64) << 20);
        let length = ((e.last_bus as u64 - e.first_bus as u64) + 1) << 20;
        let region = mapper.map(start, length).map_err(|_| Error)?;
        for bus in e.first_bus..=e.last_bus {
            for device in 0..32 {
                let mut multifunction = false;
                for function in 0..8 {
                    if function != 0 && !multifunction {
                        break;
                    }
                    let physical = e.address(bus, device, function, 0)?;
                    let registers = region
                        .subregion((physical - start) as usize, 4096)
                        .map_err(|_| Error)?;
                    let mut bytes = [0; 64];
                    for i in (0..64).step_by(4) {
                        bytes[i..i + 4].copy_from_slice(
                            &registers.read32(i).map_err(|_| Error)?.to_le_bytes(),
                        );
                    }
                    let Some(identity) = Identity::parse(&bytes)? else {
                        if function == 0 {
                            break;
                        }
                        continue;
                    };
                    if function == 0 {
                        multifunction = identity.multifunction;
                    }
                    if functions.len() >= 4096 {
                        return Err(Error);
                    }
                    functions.push(ConfigFunction {
                        function: Function {
                            segment: e.segment,
                            bus,
                            device,
                            function,
                        },
                        identity,
                        registers,
                    });
                }
            }
        }
    }
    Ok(Inventory { functions })
}
