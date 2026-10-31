//! AMD-Vi private domain with deny-default DMA and basic interrupt remapping.
//! AMD 48882-PUB rev 3.11: https://docs.amd.com/v/u/en-US/48882_3.11_IOMMU_PUB
//! The platform must quiesce *every* requester covered by the unit before takeover.
#![allow(dead_code)]
use super::MappedRegion;
use crate::kernel::{
    device::{Backend, Error},
    memory::{phys_map, vmo::Vmo},
};
use alloc::vec::Vec;
#[path = "../../../../../shared/amdvi.rs"]
pub mod encoding;
use encoding::{Buffer, Engine, Hardware};
fn device_error(error: encoding::Error) -> Error {
    match error {
        encoding::Error::Invalid => Error::Invalid,
        encoding::Error::Busy => Error::Busy,
        _ => Error::Hardware,
    }
}
struct Coherent {
    registers: MappedRegion,
    buffers: [Option<Vmo>; 7],
    retain: bool,
}
impl Coherent {
    fn new(registers: MappedRegion) -> Result<Self, Error> {
        if registers.length() < 0x4000 {
            return Err(Error::Invalid);
        }
        let mut buffers: [Option<Vmo>; 7] = core::array::from_fn(|_| None);
        for (index, slot) in buffers.iter_mut().enumerate() {
            let size = match index {
                0 => encoding::DEVICE_TABLE_BYTES,
                5 => 512 * 4096,
                _ => 4096,
            };
            *slot = Some(Vmo::create_contiguous(size).map_err(|_| Error::Quota)?);
        }
        Ok(Self {
            registers,
            buffers,
            retain: false,
        })
    }
    fn buffer(&self, buffer: Buffer) -> &Vmo {
        self.buffers[buffer as usize]
            .as_ref()
            .expect("owned IOMMU table")
    }
    fn pointer(&self, buffer: Buffer, offset: usize) -> Result<*mut u64, encoding::Error> {
        let object = self.buffer(buffer);
        if offset & 7 != 0 || offset.checked_add(8).is_none_or(|end| end > object.size()) {
            return Err(encoding::Error::Invalid);
        }
        let frame = object
            .frames()
            .get(offset / 4096)
            .ok_or(encoding::Error::Invalid)?;
        let base = phys_map::phys_to_virt(frame).ok_or(encoding::Error::Invalid)?;
        Ok((base + offset % 4096) as *mut u64)
    }
}
impl Hardware for Coherent {
    fn read_register(&self, offset: usize) -> Result<u64, encoding::Error> {
        self.registers
            .read64(offset)
            .map_err(|_| encoding::Error::Invalid)
    }
    fn write_register(&mut self, offset: usize, value: u64) -> Result<(), encoding::Error> {
        // Conservatively retain all buffers before *attempting* activation.
        if offset == encoding::CONTROL && value & 1 != 0 {
            self.retain = true;
        }
        unsafe {
            self.registers
                .write64(offset, value)
                .map_err(|_| encoding::Error::Invalid)
        }
    }
    fn physical(&self, buffer: Buffer) -> u64 {
        self.buffer(buffer)
            .frames()
            .get(0)
            .expect("contiguous IOMMU allocation") as u64
    }
    fn read(&self, buffer: Buffer, offset: usize) -> Result<u64, encoding::Error> {
        Ok(unsafe { core::ptr::read_volatile(self.pointer(buffer, offset)?) })
    }
    fn write(&mut self, buffer: Buffer, offset: usize, value: u64) -> Result<(), encoding::Error> {
        unsafe { core::ptr::write_volatile(self.pointer(buffer, offset)?, value) };
        Ok(())
    }
    fn fence(&self) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}
impl Drop for Coherent {
    fn drop(&mut self) {
        // Unit protection remains enabled even after its last driver disappears.
        // Until a platform-wide shutdown/drain protocol exists these tables live
        // until reboot, including after an indeterminate enable/completion failure.
        if self.retain {
            for slot in &mut self.buffers {
                if let Some(buffer) = slot.take() {
                    core::mem::forget(buffer);
                }
            }
        }
    }
}
/// Trusted control over the complete requester/alias assignment unit.
/// # Safety
/// Stop disables bus mastering, ATS/PASID and all device interrupt generation,
/// then halts/resets the controller with a bounded diagnosed failure. Start only
/// re-enables the verified controller, never a bridge or arbitrary PCI function.
/// Driver MMIO excludes PCI configuration, MSI-X tables and IOMMU registers.
pub unsafe trait Requester: Send + Sync {
    fn stop(&mut self) -> Result<(), Error>;
    fn start(&mut self) -> Result<(), Error>;
}
pub struct AmdVi<R: Requester> {
    engine: Engine<Coherent>,
    requester: R,
    rid: u16,
    alias: Option<u16>,
    domain: u16,
    vector: u8,
    apic: u8,
    isolated: bool,
}
impl<R: Requester> AmdVi<R> {
    /// # Safety
    /// `registers` is the exact IVRS-validated unit aperture; platform discovery
    /// proved complete requester coverage, aliases and no bypass/peer-to-peer
    /// path. Every requester on this unit is quiesced, not just this controller.
    /// Firmware DMA and interrupts have drained and ATS/PASID are disabled.
    /// Exclusive ownership of this unit lasts until reboot. No user code can
    /// alter PCI configuration, interrupt routing, or the IOMMU aperture.
    pub unsafe fn new(
        registers: MappedRegion,
        requester: R,
        rid: u16,
        alias: Option<u16>,
        domain: u16,
        vector: u8,
        apic: u8,
    ) -> Result<Self, Error> {
        if domain == 0 || alias == Some(rid) {
            return Err(Error::Invalid);
        }
        encoding::interrupt(vector, apic).map_err(device_error)?;
        let coherent = Coherent::new(registers)?;
        let mut engine = Engine::new(coherent);
        engine.enable().map_err(device_error)?;
        Ok(Self {
            engine,
            requester,
            rid,
            alias,
            domain,
            vector,
            apic,
            isolated: false,
        })
    }
    fn synchronize_assignment(&mut self, allow: bool) -> Result<(), Error> {
        for rid in [Some(self.rid), self.alias].into_iter().flatten() {
            if allow {
                self.engine.assign(rid, self.domain, self.vector, self.apic)
            } else {
                self.engine.deny(rid, self.domain)
            }
            .map_err(device_error)?;
        }
        Ok(())
    }
    pub fn check_faults(&mut self) -> Result<(), Error> {
        if let Err(error) = self.engine.check_faults() {
            let _ = self.requester.stop();
            // Cache invalidation may itself be unavailable after a hardware fault;
            // the outer Device retains leases and this backend in quarantine.
            return Err(device_error(error));
        }
        Ok(())
    }
}
// SAFETY: retained tables and completion-wait enforce DMA lifetime. The unsafe
// constructor and Requester contract establish platform ownership and quiescence.
unsafe impl<R: Requester> Backend for AmdVi<R> {
    fn isolate(&mut self) -> Result<(), Error> {
        self.requester.stop()?;
        self.synchronize_assignment(false)?;
        self.isolated = true;
        Ok(())
    }
    fn install(
        &mut self,
        iova: usize,
        object: &Vmo,
        offset: usize,
        length: usize,
        permissions: u64,
    ) -> Result<(), Error> {
        if !self.isolated {
            return Err(Error::Busy);
        }
        object
            .validate_owned_range(offset, length)
            .map_err(|_| Error::Invalid)?;
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(length / 4096)
            .map_err(|_| Error::Quota)?;
        for index in offset / 4096..(offset + length) / 4096 {
            frames.push(object.frames().get(index).ok_or(Error::Invalid)? as u64);
        }
        self.engine
            .map(iova, &frames, permissions, self.domain)
            .map_err(device_error)
    }
    fn remove(&mut self, iova: usize, length: usize) -> Result<(), Error> {
        if !self.isolated {
            return Err(Error::Busy);
        }
        self.engine
            .unmap(iova, length, self.domain)
            .map_err(device_error)
    }
    fn start(&mut self) -> Result<(), Error> {
        if !self.isolated {
            return Err(Error::Busy);
        }
        self.check_faults()?;
        self.synchronize_assignment(true)?;
        self.requester.start()?;
        self.isolated = false;
        Ok(())
    }
    fn quiesce(&mut self) -> Result<(), Error> {
        self.requester.stop()?;
        self.synchronize_assignment(false)?;
        // Invalidate + completion drains transactions against the old domain
        // (48882 §2.4.1), after trusted stop masks device interrupt generation.
        self.isolated = true;
        Ok(())
    }
}
