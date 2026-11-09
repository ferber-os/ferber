#[path = "../shared/xhci_handoff.rs"]
mod handoff;
use handoff::*;
struct Mock {
    registers: [u32; 4096],
    writes: Vec<usize>,
    irq: bool,
    dma: bool,
    stuck: Option<Error>,
    not_ready: bool,
    mask_failure: bool,
    dma_failure: bool,
}
impl Mock {
    fn new() -> Self {
        let mut h = Self {
            registers: [0; 4096],
            writes: Vec::new(),
            irq: true,
            dma: true,
            stuck: None,
            not_ready: false,
            mask_failure: false,
            dma_failure: false,
        };
        h.registers[0] = 0x1000040;
        h.registers[1] = (1 << 24) | (1 << 8) | 8;
        h.registers[4] = 0x20 << 16;
        h.registers[5] = 0x1000;
        h.registers[6] = 0x2000;
        h.registers[0x80 / 4] = 1 | (1 << 16);
        h.registers[0x84 / 4] = 0xe011;
        h
    }
}
impl Hardware for Mock {
    type Deadline = u8;
    fn read(&self, at: usize) -> Result<u32, Error> {
        self.registers.get(at / 4).copied().ok_or(Error::Layout)
    }
    fn write(&mut self, at: usize, value: u32) -> Result<(), Error> {
        self.writes.push(at);
        self.registers[at / 4] = value;
        if at == 0x80 && self.stuck != Some(Error::Ownership) {
            self.registers[at / 4] &= !(1 << 16);
        }
        if at == 0x84 && self.stuck == Some(Error::Interrupts) {
            self.registers[at / 4] = 0xe011;
        }
        if at == 0x40 {
            if value == 2 {
                if self.stuck != Some(Error::Reset) {
                    self.registers[at / 4] = 0;
                }
            } else if self.stuck != Some(Error::Halt) {
                self.registers[0x44 / 4] = 1 | if self.not_ready { 1 << 11 } else { 0 };
            }
        }
        Ok(())
    }
    fn deadline(&self) -> Result<u8, Error> {
        Ok(3)
    }
    fn expired(&self, d: &mut u8) -> Result<bool, Error> {
        *d -= 1;
        Ok(*d == 0)
    }
    fn mask_interrupts(&mut self) -> Result<(), Error> {
        if self.mask_failure {
            return Err(Error::Interrupts);
        }
        self.irq = false;
        Ok(())
    }
    fn mask_dma(&mut self) -> Result<(), Error> {
        if self.dma_failure {
            return Err(Error::Verify);
        }
        self.dma = false;
        Ok(())
    }
    fn verify(&self) -> Result<(), Error> {
        if self.stuck == Some(Error::Verify) {
            Err(Error::Verify)
        } else {
            Ok(())
        }
    }
}
#[test]
fn handoff_leaves_controller_stopped() {
    let mut h = Mock::new();
    let p = Plan::inspect(&h, 16384).unwrap();
    p.run(&mut h).unwrap();
    assert!(!h.irq && !h.dma);
    assert_eq!(h.registers[0x40 / 4], 0);
}
#[test]
fn all_partial_failures_contain_device() {
    for error in [
        Error::Ownership,
        Error::Interrupts,
        Error::Halt,
        Error::Reset,
        Error::Verify,
    ] {
        let mut h = Mock::new();
        h.stuck = Some(error);
        let p = Plan::inspect(&h, 16384).unwrap();
        assert_eq!(p.run(&mut h), Err(error));
        assert!(!h.irq && !h.dma);
    }
}
#[test]
fn invalid_layout_has_no_writes() {
    for (at, value) in [(0, 0), (4, 0), (0x14, 0xfffffff0), (0x18, 0xffffffe0)] {
        let mut h = Mock::new();
        h.registers[at / 4] = value;
        let result = Plan::inspect(&h, 16384);
        assert!(result.is_err(), "{at:x}");
        assert!(h.writes.is_empty());
    }
}

#[test]
fn duplicate_legacy_is_rejected_before_writes() {
    let mut h = Mock::new();
    h.registers[0x80 / 4] = 1 | (2 << 8);
    h.registers[0x88 / 4] = 1;
    assert!(Plan::inspect(&h, 16384).is_err());
    assert!(h.writes.is_empty());
}

#[test]
fn cnr_timeout_and_failed_masks_never_succeed() {
    for mode in 0..3 {
        let mut h = Mock::new();
        h.not_ready = mode == 0;
        h.mask_failure = mode == 1;
        h.dma_failure = mode == 2;
        let plan = Plan::inspect(&h, 16384).unwrap();
        assert!(plan.run(&mut h).is_err());
        assert!(!h.writes.contains(&0x1000), "must never ring a doorbell");
        if mode != 2 {
            assert!(!h.dma);
        }
    }
}
#[test]
fn malformed_extended_capability_walk_never_writes() {
    let mut h = Mock::new();
    h.registers[4] = 0xfff << 16;
    assert!(Plan::inspect(&h, 4096).is_err());
    assert!(h.writes.is_empty());
}

#[test]
fn register_aliases_are_rejected_before_writes() {
    for mode in 0..5 {
        let mut h = Mock::new();
        match mode {
            0 => { h.registers[4] = 0x10 << 16; h.registers[0x40 / 4] = 1; }
            1 => h.registers[5] = 0x40,
            2 => h.registers[0x80 / 4] = 1 | (1 << 8),
            3 => h.registers[6] = 0x1000,
            _ => { h.registers[4] = 0x110 << 16; h.registers[0x440 / 4] = 1; }
        }
        assert!(Plan::inspect(&h, 16384).is_err(), "mode {mode}");
        assert!(h.writes.is_empty());
    }
}

#[test]
fn protocol_payload_and_speed_records_cannot_become_capability_headers() {
    for next in [1, 4] {
        let mut h = Mock::new();
        h.registers[0x80 / 4] = 2 | (next << 8);
        h.registers[0x88 / 4] = 1 << 28;
        h.registers[(0x80 + next * 4) as usize / 4] = 1;
        assert!(Plan::inspect(&h, 16384).is_err());
        assert!(h.writes.is_empty());
    }
    let mut h = Mock::new();
    h.registers[0x80 / 4] = 2 | (5 << 8);
    h.registers[0x88 / 4] = 1 << 28;
    h.registers[0x94 / 4] = 1;
    assert!(Plan::inspect(&h, 16384).is_ok());
    assert!(h.writes.is_empty());
}

#[test]
fn unknown_and_truncated_protocol_capabilities_are_rejected() {
    let mut h = Mock::new();
    h.registers[0x80 / 4] = 3;
    assert!(Plan::inspect(&h, 16384).is_err());
    h.registers[4] = 0xfff << 16;
    h.registers[0x3ffc / 4] = 2;
    assert!(Plan::inspect(&h, 16384).is_err());
    h.registers[4] = 0xffc << 16;
    h.registers[0x3ff0 / 4] = 2;
    h.registers[0x3ff8 / 4] = 1 << 28;
    assert!(Plan::inspect(&h, 16384).is_err());
    assert!(h.writes.is_empty());
}
