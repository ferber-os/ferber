//! Bounded NS16550 queues. Register access is supplied by the delegated device mapping.
#![allow(dead_code)]
pub const RX_CAPACITY: usize = 256;
pub const TX_CAPACITY: usize = 4096;
pub trait Registers {
    fn read(&mut self, offset: usize) -> u8;
    fn write(&mut self, offset: usize, value: u8);
}
pub struct Uart {
    rx: [u8; RX_CAPACITY],
    rx_head: usize,
    rx_len: usize,
    tx: [u8; TX_CAPACITY],
    tx_head: usize,
    tx_len: usize,
    errors: u32,
    ier: u8,
}
impl Uart {
    pub const fn new() -> Self {
        Self {
            rx: [0; RX_CAPACITY],
            rx_head: 0,
            rx_len: 0,
            tx: [0; TX_CAPACITY],
            tx_head: 0,
            tx_len: 0,
            errors: 0,
            ier: 0,
        }
    }
    /// Preserve firmware's baud divisor; establish 8N1 and clean FIFOs on ownership handoff.
    pub fn start(&mut self, io: &mut impl Registers) {
        io.write(3, 3);
        io.write(1, 0);
        io.write(2, 7);
        io.write(4, 0x0b);
        self.ier = 1;
        io.write(1, self.ier);
    }
    pub fn stop(&mut self, io: &mut impl Registers) {
        self.ier = 0;
        io.write(1, 0);
    }
    pub fn queue(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > TX_CAPACITY - self.tx_len {
            return false;
        }
        for b in bytes {
            self.tx[(self.tx_head + self.tx_len) % TX_CAPACITY] = *b;
            self.tx_len += 1;
        }
        true
    }
    pub fn available(&self) -> usize {
        self.rx_len
    }
    pub fn queued(&self) -> usize {
        self.tx_len
    }
    pub fn errors(&self) -> u32 {
        self.errors
    }
    pub fn peek(&self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.rx_len);
        for (i, b) in out[..n].iter_mut().enumerate() {
            *b = self.rx[(self.rx_head + i) % RX_CAPACITY];
        }
        n
    }
    pub fn consume(&mut self, n: usize) {
        assert!(n <= self.rx_len);
        self.rx_head = (self.rx_head + n) % RX_CAPACITY;
        self.rx_len -= n;
        self.errors = 0;
    }
    /// Drain bounded work, never spin waiting for a hardware condition to change.
    pub fn pump(&mut self, io: &mut impl Registers) {
        for _ in 0..RX_CAPACITY {
            let status = io.read(5);
            if status & 0x1e != 0 {
                self.errors = self.errors.saturating_add(1);
            }
            if status & 1 == 0 {
                break;
            }
            let b = io.read(0);
            if self.rx_len == RX_CAPACITY {
                self.errors = self.errors.saturating_add(1);
            } else {
                self.rx[(self.rx_head + self.rx_len) % RX_CAPACITY] = b;
                self.rx_len += 1;
            }
        }
        for _ in 0..16 {
            if self.tx_len == 0 || io.read(5) & 0x20 == 0 {
                break;
            }
            io.write(0, self.tx[self.tx_head]);
            self.tx_head = (self.tx_head + 1) % TX_CAPACITY;
            self.tx_len -= 1;
        }
        let ier = 1 | if self.tx_len != 0 { 2 } else { 0 };
        if ier != self.ier {
            self.ier = ier;
            io.write(1, ier);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        rx: VecDeque<u8>,
        tx: Vec<u8>,
        writes: Vec<(usize, u8)>,
        ready: bool,
        errors: u8,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                rx: VecDeque::new(),
                tx: Vec::new(),
                writes: Vec::new(),
                ready: true,
                errors: 0,
            }
        }
    }
    impl Registers for Fake {
        fn read(&mut self, r: usize) -> u8 {
            match r {
                0 => self.rx.pop_front().unwrap(),
                5 => {
                    let e = self.errors;
                    self.errors = 0;
                    e | u8::from(!self.rx.is_empty()) | if self.ready { 0x20 } else { 0 }
                }
                _ => 0,
            }
        }
        fn write(&mut self, r: usize, v: u8) {
            self.writes.push((r, v));
            if r == 0 {
                self.tx.push(v);
            }
        }
    }
    #[test]
    fn bounded_rx_overrun_and_transactional_peek() {
        let mut u = Uart::new();
        let mut f = Fake::new();
        f.rx.extend((0..=255).chain(0..10));
        u.start(&mut f);
        u.pump(&mut f);
        assert_eq!(u.available(), 256);
        assert_eq!(f.rx.len(), 10);
        u.pump(&mut f);
        assert_eq!(u.errors(), 10);
        let mut b = [0; 256];
        assert_eq!(u.peek(&mut b), 256);
        assert_eq!(b[255], 255);
        assert_eq!(u.available(), 256);
        u.consume(128);
        assert_eq!(u.errors(), 0);
        assert_eq!(u.peek(&mut b), 128);
        assert_eq!(b[0], 128);
    }
    #[test]
    fn tx_waits_without_spinning_and_rearms_only_when_needed() {
        let mut u = Uart::new();
        let mut f = Fake::new();
        u.start(&mut f);
        assert!(u.queue(&[b'x'; 4096]));
        assert!(!u.queue(&[b'y']));
        f.ready = false;
        u.pump(&mut f);
        assert!(f.tx.is_empty());
        assert_eq!(f.writes.last(), Some(&(1, 3)));
        f.ready = true;
        for _ in 0..256 {
            u.pump(&mut f);
        }
        assert_eq!(f.tx, vec![b'x'; 4096]);
        assert_eq!(u.queued(), 0);
        assert_eq!(f.writes.last(), Some(&(1, 1)));
        u.stop(&mut f);
        assert_eq!(f.writes.last(), Some(&(1, 0)));
    }
    #[test]
    fn rx_wrap_and_hardware_errors_remain_visible_until_consumption() {
        let mut u = Uart::new();
        let mut f = Fake::new();
        f.rx.extend(0..200);
        u.pump(&mut f);
        u.consume(190);
        f.rx.extend(0..240);
        f.errors = 0x0e;
        u.pump(&mut f);
        assert_eq!(u.available(), 250);
        assert_eq!(u.errors(), 1);
        let mut b = [0; 256];
        u.peek(&mut b);
        assert_eq!(&b[..10], &(190..200).collect::<Vec<u8>>());
        assert_eq!(&b[10..250], &(0..240).collect::<Vec<u8>>());
    }
}
