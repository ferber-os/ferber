//! Trusted reset/control adapter. No command rings, DMA or device IRQs enabled.
#![allow(dead_code)]
use super::{acpi::Error, hpet::Clock, live_pci::ConfigFunction, mmio::MappedRegion};
pub struct Controller {
    pci: ConfigFunction,
    registers: MappedRegion,
    operational: usize,
}
impl Controller {
    /// # Safety
    /// The caller exclusively owns this xHCI PCI function and its assigned BAR.
    /// No userspace mapping may exist. The aperture must be sized and bounded.
    pub unsafe fn new(pci: ConfigFunction, registers: MappedRegion) -> Result<Self, Error> {
        if !pci.identity.is_xhci() {
            return Err(Error);
        }
        let cap = registers.read32(0).map_err(|_| Error)?;
        let operational = (cap & 255) as usize;
        let version = cap >> 16;
        if operational < 0x20 || operational & 3 != 0 || !(0x96..=0x120).contains(&version) {
            return Err(Error);
        }
        registers.subregion(operational, 0x40).map_err(|_| Error)?;
        Ok(Self {
            pci,
            registers,
            operational,
        })
    }
    /// Follow the BIOS/OS semaphore without forcing ownership after timeout.
    /// # Safety
    /// Exclusive controller ownership; a calibrated clock is running.
    pub unsafe fn firmware_handoff(&self, clock: &Clock) -> Result<(), Error> {
        let mut at = ((self.registers.read32(0x10).map_err(|_| Error)? >> 16) as usize) * 4;
        let mut visited = [0usize; 256];
        let mut n = 0;
        while at != 0 {
            if n == visited.len() || visited[..n].contains(&at) {
                return Err(Error);
            }
            visited[n] = at;
            n += 1;
            let header = self.registers.read32(at).map_err(|_| Error)?;
            if header & 255 == 1 {
                self.registers
                    .write32(at, header | (1 << 24))
                    .map_err(|_| Error)?;
                clock.wait_until(1_000_000, || {
                    Ok(self.registers.read32(at).map_err(|_| Error)? & (1 << 16) == 0)
                })?;
                let control = self.registers.read32(at + 4).map_err(|_| Error)?;
                // Disable every defined SMI enable, preserve reserved low bits,
                // and avoid accidental W1C writes to status bits.
                self.registers
                    .write32(at + 4, control & 0x000e_1fee)
                    .map_err(|_| Error)?;
                if self.registers.read32(at + 4).map_err(|_| Error)? & 0xe011 != 0 {
                    return Err(Error);
                }
                return Ok(());
            }
            let next = ((header >> 8) & 255) as usize;
            if next == 0 {
                break;
            }
            at = at.checked_add(next * 4).ok_or(Error)?;
        }
        Ok(())
    }
    /// Halt before BME clear, then reset while already halted. A timeout leaves
    /// interrupts and BME masked and never admits this requester to a domain.
    /// # Safety
    /// Firmware handoff completed; exclusive ownership and no user MMIO mapping.
    pub unsafe fn halt_and_reset(&self, clock: &Clock) -> Result<(), Error> {
        let command = self.registers.read32(self.operational).map_err(|_| Error)?;
        self.registers
            .write32(self.operational, command & !((1 << 0) | (1 << 2)))
            .map_err(|_| Error)?;
        let halted = clock.wait_until(1_000_000, || {
            Ok(self
                .registers
                .read32(self.operational + 4)
                .map_err(|_| Error)?
                & 1
                != 0)
        });
        // Mask BME even on a halt failure; a failed controller is quarantined.
        self.pci.mask_bus_mastering()?;
        halted?;
        clock.wait_until(1_000_000, || {
            Ok(self
                .registers
                .read32(self.operational + 4)
                .map_err(|_| Error)?
                & (1 << 11)
                == 0)
        })?;
        self.registers
            .write32(self.operational, 2)
            .map_err(|_| Error)?;
        clock.wait_until(1_000_000, || {
            Ok(self.registers.read32(self.operational).map_err(|_| Error)? & 2 == 0)
        })?;
        clock.wait_until(1_000_000, || {
            Ok(self
                .registers
                .read32(self.operational + 4)
                .map_err(|_| Error)?
                & (1 << 11)
                == 0)
        })?;
        if self
            .registers
            .read32(self.operational + 4)
            .map_err(|_| Error)?
            & 1
            == 0
            || self.pci.read16(4)? & 4 != 0
        {
            return Err(Error);
        }
        Ok(())
    }
}

#[path = "../../../../../shared/xhci_handoff.rs"]
pub mod handoff;
struct Adapter<'a> {
    controller: &'a Controller,
    clock: &'a Clock,
    bar: u64,
    identity: u32,
    class_revision: u32,
}
impl handoff::Hardware for Adapter<'_> {
    type Deadline = super::hpet::Deadline;
    fn read(&self, at: usize) -> Result<u32, handoff::Error> {
        self.controller
            .registers
            .read32(at)
            .map_err(|_| handoff::Error::Layout)
    }
    fn write(&mut self, at: usize, value: u32) -> Result<(), handoff::Error> {
        unsafe { self.controller.registers.write32(at, value) }.map_err(|_| handoff::Error::Layout)
    }
    fn deadline(&self) -> Result<Self::Deadline, handoff::Error> {
        self.clock
            .deadline(1_000_000)
            .map_err(|_| handoff::Error::Verify)
    }
    fn expired(&self, deadline: &mut Self::Deadline) -> Result<bool, handoff::Error> {
        self.clock
            .expired(deadline)
            .map_err(|_| handoff::Error::Verify)
    }
    fn mask_interrupts(&mut self) -> Result<(), handoff::Error> {
        unsafe { self.controller.pci.mask_interrupts() }.map_err(|_| handoff::Error::Interrupts)
    }
    fn mask_dma(&mut self) -> Result<(), handoff::Error> {
        unsafe { self.controller.pci.clear_bus_mastering() }.map_err(|_| handoff::Error::Verify)
    }
    fn verify(&self) -> Result<(), handoff::Error> {
        let pci = self.controller.pci;
        let low = pci.read32(0x10).map_err(|_| handoff::Error::Verify)?;
        let high = if low & 6 == 4 {
            pci.read32(0x14).map_err(|_| handoff::Error::Verify)?
        } else {
            0
        };
        if pci.read32(0).map_err(|_| handoff::Error::Verify)? != self.identity
            || pci.read32(8).map_err(|_| handoff::Error::Verify)? != self.class_revision
            || (low as u64 | ((high as u64) << 32)) != self.bar
            || pci.read16(4).map_err(|_| handoff::Error::Verify)? & 0x404 != 0x400
        {
            return Err(handoff::Error::Verify);
        }
        Ok(())
    }
}
impl Controller {
    pub unsafe fn diagnostic_handoff(
        &self,
        clock: &Clock,
        bar: u64,
        identity: u32,
        class_revision: u32,
    ) -> Result<(), handoff::Error> {
        let mut adapter = Adapter {
            controller: self,
            clock,
            bar,
            identity,
            class_revision,
        };
        let plan = handoff::Plan::inspect(&adapter, self.registers.length())?;
        plan.run(&mut adapter)
    }
}
