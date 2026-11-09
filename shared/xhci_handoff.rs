//! Bounded selected-controller firmware handoff. No DMA or interrupt activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Layout,
    Ownership,
    Interrupts,
    Halt,
    Reset,
    Verify,
}
pub trait Hardware {
    type Deadline;
    fn read(&self, at: usize) -> Result<u32, Error>;
    fn write(&mut self, at: usize, value: u32) -> Result<(), Error>;
    fn deadline(&self) -> Result<Self::Deadline, Error>;
    fn expired(&self, deadline: &mut Self::Deadline) -> Result<bool, Error>;
    fn mask_interrupts(&mut self) -> Result<(), Error>;
    fn mask_dma(&mut self) -> Result<(), Error>;
    fn verify(&self) -> Result<(), Error>;
}
pub struct Plan {
    operational: usize,
    legacy: Option<usize>,
}
impl Plan {
    pub fn inspect(h: &impl Hardware, length: usize) -> Result<Self, Error> {
        let cap = h.read(0)?;
        let operational = (cap & 255) as usize;
        if operational < 0x20
            || operational & 3 != 0
            || !(0x96..=0x120).contains(&(cap >> 16))
            || operational.checked_add(0x40).is_none_or(|end| end > length)
        {
            return Err(Error::Layout);
        }
        let parameters = h.read(4)?;
        let slots = (parameters & 255) as usize;
        let ports = (parameters >> 24) as usize;
        let interrupters = ((parameters >> 8) & 0x7ff) as usize;
        let doorbell = (h.read(0x14)? & !3) as usize;
        let runtime = (h.read(0x18)? & !31) as usize;
        if slots == 0
            || ports == 0
            || interrupters == 0
            || operational
                .checked_add(0x400 + ports * 16)
                .is_none_or(|end| end > length)
            || doorbell < operational
            || doorbell
                .checked_add((slots + 1) * 4)
                .is_none_or(|end| end > length)
            || runtime < operational
            || runtime
                .checked_add(0x20 + interrupters * 32)
                .is_none_or(|end| end > length)
        {
            return Err(Error::Layout);
        }
        let regions = [
            (0, 0x20),
            (operational, operational + 0x40),
            (operational + 0x400, operational + 0x400 + ports * 16),
            (doorbell, doorbell + (slots + 1) * 4),
            (runtime, runtime + 0x20 + interrupters * 32),
        ];
        let overlaps = |a: (usize, usize), b: (usize, usize)| a.0 < b.1 && b.0 < a.1;
        for (index, region) in regions.iter().enumerate() {
            if regions[..index].iter().any(|other| overlaps(*region, *other)) {
                return Err(Error::Layout);
            }
        }
        let mut spans = [(0, 0); 256];
        let mut at = ((h.read(0x10)? >> 16) as usize) * 4;
        let mut seen = [0; 256];
        let mut count = 0;
        let mut legacy = None;
        while at != 0 {
            if at < 0x20
                || at & 3 != 0
                || count == seen.len()
                || seen[..count].contains(&at)
                || at.checked_add(4).is_none_or(|end| end > length)
            {
                return Err(Error::Layout);
            }
            let header = h.read(at)?;
            let minimum = match header & 255 {
                1 => 8,
                2 => 16,
                _ => return Err(Error::Layout),
            };
            let mut end = at.checked_add(minimum).ok_or(Error::Layout)?;
            if end > length || regions.iter().chain(spans[..count].iter()).any(|other| overlaps((at, end), *other)) {
                return Err(Error::Layout);
            }
            if header & 255 == 2 {
                let speeds = ((h.read(at + 8)? >> 28) & 15) as usize;
                end = end.checked_add(speeds * 4).ok_or(Error::Layout)?;
            }
            let span = (at, end);
            if end > length || regions.iter().chain(spans[..count].iter()).any(|other| overlaps(span, *other)) {
                return Err(Error::Layout);
            }
            seen[count] = at;
            spans[count] = span;
            count += 1;
            if header & 255 == 1 {
                if legacy.replace(at).is_some() || at.checked_add(8).is_none_or(|end| end > length)
                {
                    return Err(Error::Layout);
                }
                h.read(at + 4)?;
            }
            let next = ((header >> 8) & 255) as usize;
            at = if next == 0 {
                0
            } else {
                at.checked_add(next * 4).ok_or(Error::Layout)?
            };
        }
        Ok(Self {
            operational,
            legacy,
        })
    }
    fn wait(
        h: &impl Hardware,
        at: usize,
        mask: u32,
        expected: u32,
        error: Error,
    ) -> Result<(), Error> {
        let mut d = h.deadline().map_err(|_| error)?;
        loop {
            if h.expired(&mut d).map_err(|_| error)? {
                return Err(error);
            }
            if h.read(at).map_err(|_| error)? & mask == expected {
                return Ok(());
            }
            core::hint::spin_loop();
        }
    }
    pub fn run(&self, h: &mut impl Hardware) -> Result<(), Error> {
        let result = (|| {
            h.mask_interrupts().map_err(|_| Error::Interrupts)?;
            if let Some(at) = self.legacy {
                let header = h.read(at)?;
                h.write(at, header | (1 << 24))?;
                Self::wait(h, at, 1 << 16, 0, Error::Ownership)?;
                h.write(at + 4, h.read(at + 4)? & 0x000e_1fee)?;
                if h.read(at + 4)? & 0xe011 != 0 {
                    return Err(Error::Interrupts);
                }
            }
            let op = self.operational;
            h.write(op, h.read(op)? & !5)?;
            Self::wait(h, op + 4, 1, 1, Error::Halt)?;
            h.mask_dma()?;
            Self::wait(h, op + 4, 1 << 11, 0, Error::Reset)?;
            h.write(op, 2)?;
            Self::wait(h, op, 2, 0, Error::Reset)?;
            Self::wait(h, op + 4, 1 << 11, 0, Error::Reset)?;
            if h.read(op)? & 7 != 0 || h.read(op + 4)? & 1 == 0 {
                return Err(Error::Verify);
            }
            h.mask_interrupts()?;
            if let Some(at) = self.legacy {
                if h.read(at)? & ((1 << 24) | (1 << 16)) != 1 << 24 || h.read(at + 4)? & 0xe011 != 0
                {
                    return Err(Error::Verify);
                }
            }
            h.verify()
        })();
        if result.is_err() {
            let _ = h.mask_dma();
            let _ = h.mask_interrupts();
        }
        result
    }
}
