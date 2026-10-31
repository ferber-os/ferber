#[path = "../shared/amdvi.rs"]
mod iommu;
use iommu::*;
use std::cell::Cell;
struct Mock {
    registers: [Cell<u64>; 0x2040 / 8],
    buffers: [Vec<Cell<u64>>; 7],
    complete: bool,
    blocked_control: u64,
    commands: Vec<Command>,
    fences: Cell<usize>,
}
impl Mock {
    fn new() -> Self {
        let registers = core::array::from_fn(|_| Cell::new(0));
        let hardware = Self {
            registers,
            buffers: core::array::from_fn(|index| {
                let size = match index {
                    0 => DEVICE_TABLE_BYTES,
                    5 => 512 * PAGE,
                    _ => PAGE,
                };
                (0..size / 8).map(|_| Cell::new(0)).collect()
            }),
            complete: true,
            blocked_control: 0,
            commands: Vec::new(),
            fences: Cell::new(0),
        };
        hardware.registers[0x30 / 8].set(1 << 16);
        hardware
    }
}
impl Hardware for Mock {
    fn read_register(&self, offset: usize) -> Result<u64, Error> {
        self.registers
            .get(offset / 8)
            .map(Cell::get)
            .ok_or(Error::Invalid)
    }
    fn write_register(&mut self, offset: usize, value: u64) -> Result<(), Error> {
        if (0x60..0xe0).contains(&offset) && self.registers[offset / 8].get() & (1 << 17) != 0 {
            return Ok(());
        }
        let value = if offset == CONTROL {
            value & !self.blocked_control
        } else {
            value
        };
        self.registers
            .get(offset / 8)
            .ok_or(Error::Invalid)?
            .set(value);
        if offset == COMMAND_TAIL && self.registers[CONTROL / 8].get() & 1 != 0 {
            let at = self.registers[COMMAND_HEAD / 8].get() as usize;
            let command = Command([
                self.buffers[Buffer::Commands as usize][at / 8].get(),
                self.buffers[Buffer::Commands as usize][at / 8 + 1].get(),
            ]);
            self.commands.push(command);
            if command.0[0] >> 60 == 1 && self.complete {
                self.buffers[Buffer::Completion as usize][0].set(command.0[1]);
            }
            self.registers[COMMAND_HEAD / 8].set(value);
        }
        Ok(())
    }
    fn physical(&self, buffer: Buffer) -> u64 {
        0x1000_0000 + (buffer as u64) * 0x400000
    }
    fn read(&self, buffer: Buffer, offset: usize) -> Result<u64, Error> {
        self.buffers[buffer as usize]
            .get(offset / 8)
            .map(Cell::get)
            .ok_or(Error::Invalid)
    }
    fn write(&mut self, buffer: Buffer, offset: usize, value: u64) -> Result<(), Error> {
        self.buffers[buffer as usize]
            .get(offset / 8)
            .ok_or(Error::Invalid)?
            .set(value);
        Ok(())
    }
    fn fence(&self) {
        self.fences.set(self.fences.get() + 1);
    }
}
#[test]
fn exact_descriptors_and_commands() {
    assert_eq!(DENY, [3, 0, 1, 0]);
    assert_eq!(
        device_entry(0x12345000, 0x23456000, 9),
        Ok([0x6000000012345403, 9, 0x2000000023456001, 0])
    );
    assert_eq!(directory(0x12345000), Ok(0x6000000012345201));
    assert_eq!(leaf(0x12345000, 1), Ok(0x3000000012345001));
    assert_eq!(leaf(0x12345000, 2), Ok(0x5000000012345001));
    assert_eq!(interrupt(0x51, 0x12), Ok(0x00511201));
    assert_eq!(
        Command::invalidate_device(0x1234),
        Command([0x2000000000001234, 0])
    );
    assert_eq!(
        Command::invalidate_interrupt(0x1234),
        Command([0x5000000000001234, 0])
    );
    assert_eq!(
        Command::invalidate_domain(9),
        Command([0x3000000900000000, 0x7ffffffffffff003])
    );
    assert_eq!(
        Command::completion(0x12345008, 7),
        Ok(Command([0x100000001234500d, 7]))
    );
}
#[test]
fn reject_reserved_fields_and_unaligned_addresses() {
    for address in [0, 1, 0x1001, 1 << 52] {
        assert_eq!(leaf(address, 3), Err(Error::Invalid));
    }
    for permissions in [0, 4, 8, u64::MAX] {
        assert_eq!(leaf(0x1000, permissions), Err(Error::Invalid));
    }
    assert_eq!(device_entry(0x1000, 0x2000, 0), Err(Error::Invalid));
    assert_eq!(interrupt(31, 0), Err(Error::Invalid));
    assert_eq!(Command::completion(0x1001, 1), Err(Error::Invalid));
    assert_eq!(Command::completion(0x1000, 0), Err(Error::Invalid));
}
fn enabled() -> Engine<Mock> {
    let mut engine = Engine::new(Mock::new());
    unsafe { engine.enable().unwrap() };
    engine
}
#[test]
fn initialization_denies_every_requester_and_interrupt() {
    let engine = enabled();
    for rid in 0..65536 {
        for (word, value) in DENY.into_iter().enumerate() {
            assert_eq!(
                engine.hardware().read(Buffer::Devices, rid * 32 + word * 8),
                Ok(value)
            );
        }
    }
    assert_eq!(engine.hardware().read_register(0), Ok(0x100001ff));
    assert_eq!(engine.hardware().read_register(CONTROL), Ok(ENABLE));
    assert_eq!(engine.hardware().read(Buffer::Interrupts, 0), Ok(0));
    assert_eq!(
        engine.hardware().read(Buffer::Root, 0),
        directory(engine.hardware().physical(Buffer::Leaves))
    );
}
#[test]
fn firmware_active_is_not_overwritten() {
    let hardware = Mock::new();
    hardware.registers[CONTROL / 8].set(1);
    let mut engine = Engine::new(hardware);
    assert_eq!(unsafe { engine.enable() }, Err(Error::Busy));
    assert!(!engine.enabled());
    assert!(engine.hardware().commands.is_empty());
    let hardware = Mock::new();
    hardware.registers[CONTROL / 8].set(1 << 10);
    let mut engine = Engine::new(hardware);
    unsafe { engine.enable().unwrap() };
}
#[test]
fn assignment_mapping_and_reclamation_are_synchronized() {
    let mut engine = enabled();
    engine.map(0x1000, &[0x2000, 0x8000], 3, 7).unwrap();
    assert_eq!(engine.map(0x2000, &[0x9000], 1, 7), Err(Error::Busy));
    engine.assign(0x1234, 7, 0x51, 0).unwrap();
    assert_eq!(engine.hardware().read(Buffer::Interrupts, 0), Ok(0x510001));
    engine.deny(0x1234, 7).unwrap();
    let commands = &engine.hardware().commands;
    assert_eq!(
        &commands[commands.len() - 4..commands.len() - 1],
        &[
            Command::invalidate_device(0x1234),
            Command::invalidate_interrupt(0x1234),
            Command::invalidate_domain(7)
        ]
    );
    engine.unmap(0x1000, 8192, 7).unwrap();
    assert_eq!(engine.hardware().read(Buffer::Leaves, 8), Ok(0));
    assert_eq!(engine.unmap(0x1000, 8192, 7), Err(Error::Invalid));
    assert_eq!(engine.map(IOVA_LIMIT, &[0x1000], 3, 7), Err(Error::Invalid));
}
#[test]
fn no_partial_publish_on_invalid_mapping() {
    let mut engine = enabled();
    let count = engine.hardware().commands.len();
    assert_eq!(
        engine.map(0x1000, &[0x2000, 0x3333], 3, 7),
        Err(Error::Invalid)
    );
    assert_eq!(engine.hardware().read(Buffer::Leaves, 8), Ok(0));
    assert_eq!(engine.hardware().commands.len(), count);
}
#[test]
fn lost_completion_poisoned_and_blocks_reuse() {
    let mut engine = enabled();
    engine.hardware_mut().complete = false;
    assert_eq!(engine.map(0x1000, &[0x2000], 3, 7), Err(Error::Timeout));
    assert!(engine.poisoned());
    assert_eq!(engine.unmap(0x1000, 4096, 7), Err(Error::Fault));
    assert_ne!(engine.hardware().read(Buffer::Leaves, 8), Ok(0));
}
#[test]
fn pending_faults_quarantine_instead_of_clearing_logs() {
    let mut engine = enabled();
    engine.hardware().registers[EVENT_TAIL / 8].set(16);
    assert_eq!(engine.check_faults(), Err(Error::Fault));
    assert!(engine.poisoned());
    assert_eq!(engine.hardware().read_register(EVENT_HEAD), Ok(0));
}
#[test]
fn command_ring_wrap_does_not_drop_invalidations() {
    let mut engine = enabled();
    for _ in 0..400 {
        engine.deny(0x1234, 7).unwrap();
    }
    assert_eq!(engine.hardware().commands.len(), 1601);
    assert_eq!(
        engine.hardware().read_register(COMMAND_HEAD),
        engine.hardware().read_register(COMMAND_TAIL)
    );
}

#[test]
fn completed_commands_do_not_hide_faults() {
    for event in [false, true] {
        let mut engine = enabled();
        if event {
            engine.hardware().registers[EVENT_TAIL / 8].set(16);
        } else {
            engine.hardware().registers[STATUS / 8].set(2);
        }
        assert_eq!(engine.assign(0x1234, 7, 0x51, 0), Err(Error::Fault));
        assert!(engine.poisoned());
        assert_eq!(engine.map(0x1000, &[0x2000], 3, 7), Err(Error::Fault));
    }
}
#[test]
fn smi_passthrough_units_are_rejected_before_enable() {
    for features in [0, 2 << 16, 3 << 16, (1 << 16) | (5 << 18)] {
        let hardware = Mock::new();
        hardware.registers[0x30 / 8].set(features);
        let mut engine = Engine::new(hardware);
        assert_eq!(unsafe { engine.enable() }, Err(Error::Invalid));
        assert_eq!(engine.hardware().read_register(CONTROL), Ok(0));
    }
    let mut engine = enabled();
    assert_ne!(
        engine.hardware().read_register(CONTROL).unwrap() & (1 << 22),
        0
    );
    assert_eq!(
        engine.hardware().read_register(0x60).unwrap() & (1 << 16),
        0
    );
    assert_eq!(engine.check_faults(), Ok(()));
}

#[test]
fn locked_smi_allowlist_and_ignored_filter_enable_fail_closed() {
    let hardware = Mock::new();
    hardware.registers[0x60 / 8].set((1 << 17) | (1 << 16) | 0x1234);
    let mut engine = Engine::new(hardware);
    assert_eq!(unsafe { engine.enable() }, Err(Error::Busy));
    assert_eq!(engine.hardware().read_register(CONTROL), Ok(0));
    let mut hardware = Mock::new();
    hardware.blocked_control = 1 << 22;
    let mut engine = Engine::new(hardware);
    assert_eq!(unsafe { engine.enable() }, Err(Error::Fault));
    assert!(engine.enabled() && engine.poisoned());
}
