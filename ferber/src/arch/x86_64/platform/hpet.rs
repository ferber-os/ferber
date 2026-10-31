//! HPET-backed deadlines for bootstrap hardware protocols; no TSC calibration guesses.
#![allow(dead_code)]
use super::{acpi::Error, mmio::MappedRegion};
#[derive(Clone, Copy)]
pub struct Clock {
    registers: MappedRegion,
    period_fs: u64,
}
pub struct Deadline {
    start: u64,
    ticks: u64,
    last: u64,
    stalled: u32,
}
/// Convert a finite interval using HPET's specified femtoseconds/tick field.
pub fn ticks_for_us(period_fs: u64, microseconds: u64) -> Result<u64, Error> {
    if period_fs == 0 || period_fs > 100_000_000 || microseconds == 0 {
        return Err(Error);
    }
    let ticks = (microseconds as u128 * 1_000_000_000u128).div_ceil(period_fs as u128);
    if ticks > i64::MAX as u128 {
        return Err(Error);
    }
    Ok(ticks as u64)
}
impl Clock {
    /// # Safety
    /// Exclusive kernel ownership of a validated, uncached HPET aperture. No
    /// firmware agent or driver may use its counter or timer routes concurrently.
    pub unsafe fn initialize(registers: MappedRegion) -> Result<Self, Error> {
        let cap = registers.read64(0).map_err(|_| Error)?;
        let period_fs = cap >> 32;
        ticks_for_us(period_fs, 1)?;
        if cap & (1 << 13) == 0 || cap & 255 == 0 {
            return Err(Error);
        }
        let timers = ((cap >> 8) & 31) + 1;
        registers.write64(0x10, 0).map_err(|_| Error)?;
        for index in 0..timers as usize {
            let at = 0x100 + index * 0x20;
            let config = registers.read64(at).map_err(|_| Error)?;
            registers
                .write64(at, config & !((1 << 2) | (1 << 14)))
                .map_err(|_| Error)?;
        }
        registers.write64(0xf0, 0).map_err(|_| Error)?;
        registers.write64(0x10, 1).map_err(|_| Error)?;
        if registers.read64(0x10).map_err(|_| Error)? & 3 != 1 {
            return Err(Error);
        }
        Ok(Self {
            registers,
            period_fs,
        })
    }
    pub fn now(&self) -> Result<u64, Error> {
        self.registers.read64(0xf0).map_err(|_| Error)
    }
    pub fn deadline(&self, microseconds: u64) -> Result<Deadline, Error> {
        let start = self.now()?;
        Ok(Deadline {
            start,
            ticks: ticks_for_us(self.period_fs, microseconds)?,
            last: start,
            stalled: 0,
        })
    }
    pub fn expired(&self, d: &mut Deadline) -> Result<bool, Error> {
        let now = self.now()?;
        // This is a broken-clock watchdog, not a timing estimate. Real timeout
        // decisions use only HPET ticks; a stopped clock fails closed.
        if now == d.last {
            d.stalled = d.stalled.checked_add(1).ok_or(Error)?;
            if d.stalled >= 1_000_000 {
                return Err(Error);
            }
        } else {
            d.last = now;
            d.stalled = 0;
        }
        Ok(now.wrapping_sub(d.start) >= d.ticks)
    }
    pub fn wait_us(&self, microseconds: u64) -> Result<(), Error> {
        let mut d = self.deadline(microseconds)?;
        while !self.expired(&mut d)? {
            core::hint::spin_loop();
        }
        Ok(())
    }
    pub fn wait_until(
        &self,
        microseconds: u64,
        mut condition: impl FnMut() -> Result<bool, Error>,
    ) -> Result<(), Error> {
        let mut d = self.deadline(microseconds)?;
        loop {
            if condition()? {
                return Ok(());
            }
            if self.expired(&mut d)? {
                return Err(Error);
            }
            core::hint::spin_loop();
        }
    }
}
