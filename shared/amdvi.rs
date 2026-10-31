//! AMD 48882 rev 3.11: DTE table 7, PTE tables 17/18, IRTE table 20,
//! commands sections 2.4.1–2.4.5. No ATS, PASID, guest mode or pass-through.
#![allow(dead_code)]
pub const PAGE: usize = 4096;
pub const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
pub const DEVICE_COUNT: usize = 65536;
pub const DEVICE_TABLE_BYTES: usize = DEVICE_COUNT * 32;
pub const IOVA_LIMIT: usize = 1 << 30;
pub const IR: u64 = 1 << 61;
pub const IW: u64 = 1 << 62;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Busy,
    Fault,
    Timeout,
}
fn page(address: u64) -> Result<u64, Error> {
    if address == 0 || address & !ADDRESS_MASK != 0 {
        Err(Error::Invalid)
    } else {
        Ok(address)
    }
}
/// All-zero DTEs bypass translation. This valid entry instead denies DMA,
/// port I/O and remapped interrupts. SMIs require the separate deny-all filter.
pub const DENY: [u64; 4] = [3, 0, 1, 0];
pub fn device_entry(root: u64, interrupt_table: u64, domain: u16) -> Result<[u64; 4], Error> {
    let root = page(root)?;
    let interrupt_table = page(interrupt_table)?;
    if domain == 0 {
        return Err(Error::Invalid);
    }
    // Mode=2 gives a 30-bit IOVA; IV=1, IntCtl=10b, IntTabLen=0
    // admits only index zero in the basic-format interrupt table.
    Ok([
        root | 3 | (2 << 9) | IR | IW,
        domain as u64,
        interrupt_table | 1 | (2 << 60),
        0,
    ])
}
pub fn directory(next: u64) -> Result<u64, Error> {
    Ok(page(next)? | 1 | (1 << 9) | IR | IW)
}
pub fn leaf(physical: u64, permissions: u64) -> Result<u64, Error> {
    let physical = page(physical)?;
    if permissions == 0 || permissions & !3 != 0 {
        return Err(Error::Invalid);
    }
    Ok(physical
        | 1
        | (1 << 60)
        | if permissions & 1 != 0 { IR } else { 0 }
        | if permissions & 2 != 0 { IW } else { 0 })
}
pub fn interrupt(vector: u8, apic: u8) -> Result<u32, Error> {
    if vector < 32 {
        return Err(Error::Invalid);
    }
    Ok(1 | ((apic as u32) << 8) | ((vector as u32) << 16))
}
pub fn device_table_base(physical: u64) -> Result<u64, Error> {
    Ok(page(physical)? | 0x1ff)
}
pub fn ring_base(physical: u64) -> Result<u64, Error> {
    Ok(page(physical)? | (8 << 56))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command(pub [u64; 2]);
impl Command {
    pub const fn invalidate_device(rid: u16) -> Self {
        Self([(2 << 60) | rid as u64, 0])
    }
    pub const fn invalidate_interrupt(rid: u16) -> Self {
        Self([(5 << 60) | rid as u64, 0])
    }
    pub const fn invalidate_domain(domain: u16) -> Self {
        // S=PDE=1, GN=0, address=7_FFFF_FFFF_FFFFh invalidates the entire domain.
        Self([(3 << 60) | ((domain as u64) << 32), 0x7fff_ffff_ffff_f003])
    }
    pub fn completion(physical: u64, sequence: u64) -> Result<Self, Error> {
        if physical == 0 || physical & 7 != 0 || physical >= 1 << 52 || sequence == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self([(1 << 60) | physical | 5, sequence]))
    }
}
pub const CONTROL: usize = 0x18;
pub const STATUS: usize = 0x2020;
pub const COMMAND_HEAD: usize = 0x2000;
pub const COMMAND_TAIL: usize = 0x2008;
pub const EVENT_HEAD: usize = 0x2010;
pub const EVENT_TAIL: usize = 0x2018;
pub const ENABLE: u64 = 1 | (1 << 2) | (1 << 10) | (1 << 12) | (1 << 22) | (1 << 24);
/// Coherent memory identifiers are private to a trusted owner. Implementations
/// retain all buffers while enabled, including on failed completion/quiescence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Buffer {
    Devices,
    Commands,
    Events,
    Completion,
    Root,
    Leaves,
    Interrupts,
}
pub trait Hardware {
    fn read_register(&self, offset: usize) -> Result<u64, Error>;
    fn write_register(&mut self, offset: usize, value: u64) -> Result<(), Error>;
    fn physical(&self, buffer: Buffer) -> u64;
    fn read(&self, buffer: Buffer, offset: usize) -> Result<u64, Error>;
    fn write(&mut self, buffer: Buffer, offset: usize, value: u64) -> Result<(), Error>;
    fn fence(&self);
}
pub struct Engine<H: Hardware> {
    hardware: H,
    tail: usize,
    sequence: u64,
    poisoned: bool,
    enabled: bool,
}
impl<H: Hardware> Engine<H> {
    pub fn new(hardware: H) -> Self {
        Self {
            hardware,
            tail: 0,
            sequence: 0,
            poisoned: false,
            enabled: false,
        }
    }
    pub fn hardware(&self) -> &H {
        &self.hardware
    }
    pub fn hardware_mut(&mut self) -> &mut H {
        &mut self.hardware
    }
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    /// # Safety
    /// Every requester covered by this IOMMU is quiesced with bus mastering,
    /// ATS and device interrupt generation disabled. Firmware ownership is over;
    /// no other CPU/driver can touch this unit or its private coherent buffers.
    pub unsafe fn enable(&mut self) -> Result<(), Error> {
        if self.enabled || self.poisoned {
            return Err(Error::Busy);
        }
        if self.hardware.read_register(CONTROL)? & !(1 << 10) != 0
            || self.hardware.read_register(STATUS)? & 0x18 != 0
        {
            return Err(Error::Busy);
        }
        // SMIs bypass DTE interrupt controls. Reject units without an enforceable filter.
        let features = self.hardware.read_register(0x30)?;
        let count = (features >> 18) & 7;
        if (features >> 16) & 3 != 1 || count > 4 {
            return Err(Error::Invalid);
        }
        for index in 0..(1usize << count) {
            let register = 0x60 + index * 8;
            self.hardware.write_register(register, 0)?;
            // A firmware-locked valid allow entry cannot be taken over safely.
            if self.hardware.read_register(register)? & (1 << 16) != 0 {
                return Err(Error::Busy);
            }
        }
        for rid in 0..DEVICE_COUNT {
            for (word, value) in DENY.into_iter().enumerate() {
                self.hardware
                    .write(Buffer::Devices, rid * 32 + word * 8, value)?;
            }
        }
        for buffer in [
            Buffer::Commands,
            Buffer::Events,
            Buffer::Completion,
            Buffer::Root,
            Buffer::Leaves,
            Buffer::Interrupts,
        ] {
            let size = if buffer == Buffer::Leaves {
                512 * PAGE
            } else {
                PAGE
            };
            for at in (0..size).step_by(8) {
                self.hardware.write(buffer, at, 0)?;
            }
        }
        // Every subordinate 4KiB table is preallocated. No mapping operation allocates.
        let leaves = self.hardware.physical(Buffer::Leaves);
        for index in 0..512 {
            self.hardware.write(
                Buffer::Root,
                index * 8,
                directory(leaves + (index * PAGE) as u64)?,
            )?;
        }
        self.hardware.write_register(
            0,
            device_table_base(self.hardware.physical(Buffer::Devices))?,
        )?;
        self.hardware
            .write_register(8, ring_base(self.hardware.physical(Buffer::Commands))?)?;
        self.hardware
            .write_register(0x10, ring_base(self.hardware.physical(Buffer::Events))?)?;
        // Disable exclusion ranges; every byte goes through the deny-default DTEs.
        self.hardware.write_register(0x20, 0)?;
        self.hardware.write_register(0x28, 0)?;
        for register in [COMMAND_HEAD, COMMAND_TAIL, EVENT_HEAD, EVENT_TAIL] {
            self.hardware.write_register(register, 0)?;
        }
        self.hardware.fence();
        // Once this write is attempted the caller must retain buffers even on error.
        self.enabled = true;
        let result = self.hardware.write_register(CONTROL, ENABLE).and_then(|_| {
            if self.hardware.read_register(CONTROL)? & ENABLE != ENABLE {
                return Err(Error::Fault);
            }
            self.complete()
        });
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn ensure(&self) -> Result<(), Error> {
        if self.poisoned || !self.enabled {
            Err(Error::Fault)
        } else {
            Ok(())
        }
    }
    fn submit(&mut self, command: Command) -> Result<(), Error> {
        let next = (self.tail + 16) & (PAGE - 1);
        let head = self.hardware.read_register(COMMAND_HEAD)?;
        if head >= PAGE as u64 || head & 15 != 0 {
            return Err(Error::Fault);
        }
        if next == head as usize {
            return Err(Error::Busy);
        }
        self.hardware
            .write(Buffer::Commands, self.tail, command.0[0])?;
        self.hardware
            .write(Buffer::Commands, self.tail + 8, command.0[1])?;
        self.hardware.fence();
        self.hardware.write_register(COMMAND_TAIL, next as u64)?;
        self.tail = next;
        Ok(())
    }
    fn complete(&mut self) -> Result<(), Error> {
        let sequence = self.sequence.checked_add(1).ok_or(Error::Fault)?;
        self.sequence = sequence;
        self.submit(Command::completion(
            self.hardware.physical(Buffer::Completion),
            sequence,
        )?)?;
        // shortcut: fixed poll budget can reject slow hardware; use the platform clock before activation.
        for _ in 0..1_000_000 {
            self.hardware.fence();
            self.check_faults()?;
            if self.hardware.read(Buffer::Completion, 0)? == sequence {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::Timeout)
    }
    fn synchronize(&mut self, rid: u16, domain: u16) -> Result<(), Error> {
        let result = self
            .submit(Command::invalidate_device(rid))
            .and_then(|_| self.submit(Command::invalidate_interrupt(rid)))
            .and_then(|_| self.submit(Command::invalidate_domain(domain)))
            .and_then(|_| self.complete());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    /// Caller has masked/reset this requester; it may still have old cached DMA.
    pub fn deny(&mut self, rid: u16, domain: u16) -> Result<(), Error> {
        self.ensure()?;
        for (word, value) in DENY.into_iter().enumerate() {
            self.hardware
                .write(Buffer::Devices, rid as usize * 32 + word * 8, value)?;
        }
        self.hardware.fence();
        self.synchronize(rid, domain)
    }
    pub fn assign(&mut self, rid: u16, domain: u16, vector: u8, apic: u8) -> Result<(), Error> {
        self.ensure()?;
        let entry = device_entry(
            self.hardware.physical(Buffer::Root),
            self.hardware.physical(Buffer::Interrupts),
            domain,
        )?;
        self.hardware
            .write(Buffer::Interrupts, 0, interrupt(vector, apic)? as u64)?;
        // Publish word zero last, following DTE update ordering. Requester stays masked.
        for word in [1, 2, 3, 0] {
            self.hardware
                .write(Buffer::Devices, rid as usize * 32 + word * 8, entry[word])?;
        }
        self.hardware.fence();
        self.synchronize(rid, domain)
    }
    pub fn map(
        &mut self,
        iova: usize,
        frames: &[u64],
        permissions: u64,
        domain: u16,
    ) -> Result<(), Error> {
        self.ensure()?;
        let bytes = frames.len().checked_mul(PAGE).ok_or(Error::Invalid)?;
        if frames.is_empty()
            || iova == 0
            || iova & (PAGE - 1) != 0
            || iova.checked_add(bytes).is_none_or(|end| end > IOVA_LIMIT)
        {
            return Err(Error::Invalid);
        }
        for (index, frame) in frames.iter().enumerate() {
            leaf(*frame, permissions)?;
            if self
                .hardware
                .read(Buffer::Leaves, (iova / PAGE + index) * 8)?
                != 0
            {
                return Err(Error::Busy);
            }
        }
        for (index, frame) in frames.iter().enumerate() {
            self.hardware.write(
                Buffer::Leaves,
                (iova / PAGE + index) * 8,
                leaf(*frame, permissions)?,
            )?;
        }
        self.hardware.fence();
        let result = self
            .submit(Command::invalidate_domain(domain))
            .and_then(|_| self.complete());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub fn unmap(&mut self, iova: usize, length: usize, domain: u16) -> Result<(), Error> {
        self.ensure()?;
        if length == 0
            || (iova | length) & (PAGE - 1) != 0
            || iova.checked_add(length).is_none_or(|end| end > IOVA_LIMIT)
        {
            return Err(Error::Invalid);
        }
        for index in iova / PAGE..(iova + length) / PAGE {
            if self.hardware.read(Buffer::Leaves, index * 8)? == 0 {
                return Err(Error::Invalid);
            }
        }
        for index in iova / PAGE..(iova + length) / PAGE {
            self.hardware.write(Buffer::Leaves, index * 8, 0)?;
        }
        self.hardware.fence();
        let result = self
            .submit(Command::invalidate_domain(domain))
            .and_then(|_| self.complete());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    /// Faults are never hidden: any pending event quarantines the assignment.
    pub fn check_faults(&mut self) -> Result<(), Error> {
        self.ensure()?;
        let result = (|| {
            if self.hardware.read_register(STATUS)? & 3 != 0
                || self.hardware.read_register(EVENT_HEAD)?
                    != self.hardware.read_register(EVENT_TAIL)?
            {
                return Err(Error::Fault);
            }
            Ok(())
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
