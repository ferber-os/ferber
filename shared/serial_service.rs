//! Serial console requests reserve no heap memory and commit only after reply delivery.
#![allow(dead_code)]
use super::console_protocol as p;
use super::uart16550::{Registers, Uart, TX_CAPACITY};
pub const INPUT: u64 = 1;
pub const OUTPUT: u64 = 2;
pub const OVERRUN: i32 = -75;
#[derive(Debug)]
enum Action {
    None,
    Read(usize),
    Write(usize),
    ReportErrors,
}
/// Keep the request buffer unchanged and do not pump between preparing and committing.
/// A failed channel send discards this ticket without consuming bytes or accepting output.
#[derive(Debug)]
pub struct Reply {
    pub len: usize,
    request: p::Header,
    action: Action,
}
pub struct SerialService {
    uart: Uart,
}
impl SerialService {
    pub const fn new() -> Self {
        Self { uart: Uart::new() }
    }
    pub fn start(&mut self, io: &mut impl Registers) {
        self.uart.start(io);
    }
    pub fn stop(&mut self, io: &mut impl Registers) {
        self.uart.stop(io);
    }
    pub fn pump(&mut self, io: &mut impl Registers) {
        self.uart.pump(io);
    }
    pub fn queued_output(&self) -> usize {
        self.uart.queued()
    }
    pub fn available_input(&self) -> usize {
        self.uart.available()
    }
    /// Errors indicate possible loss/corruption, not an exact count of lost hardware bytes.
    pub fn has_input_error(&self) -> bool {
        self.uart.errors() != 0
    }
    pub fn prepare(
        &self,
        bytes: &[u8],
        out: &mut [u8; p::HEADER + p::MAX_INPUT],
    ) -> Result<Option<Reply>, p::ProtocolError> {
        let request = p::request(bytes)?;
        let mut h = request;
        h.argument = 0;
        h.value = 0;
        let (action, n) = match request.operation {
            p::READ if self.has_input_error() => {
                h.status = OVERRUN;
                (Action::ReportErrors, 0)
            }
            p::READ => {
                if self.uart.available() == 0 {
                    return Ok(None);
                }
                let length = (request.argument as usize).min(self.uart.available());
                let n = self.uart.peek(&mut out[p::HEADER..p::HEADER + length]);
                h.argument = n as u32;
                (Action::Read(n), n)
            }
            p::WRITE => {
                let len = bytes.len() - p::HEADER;
                if len > TX_CAPACITY - self.uart.queued() {
                    return Ok(None);
                }
                h.argument = len as u32;
                (Action::Write(len), 0)
            }
            p::INFO => {
                h.value = INPUT | OUTPUT;
                (Action::None, 0)
            }
            p::COLORS => {
                h.status = -95;
                (Action::None, 0)
            }
            _ => return Err(p::ProtocolError),
        };
        let mut header = [0; p::HEADER];
        p::encode(h, &mut header);
        out[..p::HEADER].copy_from_slice(&header);
        Ok(Some(Reply {
            len: p::HEADER + n,
            request,
            action,
        }))
    }
    /// The caller owns one outstanding client request and commits before pumping again.
    pub fn commit(&mut self, reply: Reply, bytes: &[u8]) -> Result<(), p::ProtocolError> {
        if p::request(bytes)? != reply.request {
            return Err(p::ProtocolError);
        }
        match reply.action {
            Action::None => {}
            Action::Read(n) => {
                if n > self.uart.available() {
                    return Err(p::ProtocolError);
                }
                self.uart.consume(n);
            }
            Action::Write(n) => {
                if bytes.len() != p::HEADER + n || !self.uart.queue(&bytes[p::HEADER..]) {
                    return Err(p::ProtocolError);
                }
            }
            Action::ReportErrors => self.uart.consume(0),
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        rx: VecDeque<u8>,
        tx: Vec<u8>,
        ready: bool,
        error: u8,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                rx: VecDeque::new(),
                tx: Vec::new(),
                ready: true,
                error: 0,
            }
        }
    }
    impl Registers for Fake {
        fn read(&mut self, n: usize) -> u8 {
            match n {
                0 => self.rx.pop_front().unwrap(),
                5 => {
                    let e = self.error;
                    self.error = 0;
                    e | u8::from(!self.rx.is_empty()) | if self.ready { 0x20 } else { 0 }
                }
                _ => 0,
            }
        }
        fn write(&mut self, n: usize, b: u8) {
            if n == 0 {
                self.tx.push(b);
            }
        }
    }
    fn request(op: u16, argument: u32, payload: &[u8]) -> Vec<u8> {
        let mut b = vec![0; p::HEADER + payload.len()];
        let mut h = [0; p::HEADER];
        p::encode(
            p::Header {
                operation: op,
                request: 1,
                status: 0,
                argument,
                value: 0,
            },
            &mut h,
        );
        b[..p::HEADER].copy_from_slice(&h);
        b[p::HEADER..].copy_from_slice(payload);
        b
    }
    #[test]
    fn empty_read_defers_and_failed_delivery_retains_input() {
        let mut s = SerialService::new();
        let mut f = Fake::new();
        let b = request(p::READ, 4, &[]);
        let mut out = [0; p::HEADER + p::MAX_INPUT];
        assert!(s.prepare(&b, &mut out).unwrap().is_none());
        f.rx.extend(b"hello");
        s.pump(&mut f);
        let r = s.prepare(&b, &mut out).unwrap().unwrap();
        assert_eq!(r.len, p::HEADER + 4);
        assert_eq!(&out[p::HEADER..r.len], b"hell");
        assert_eq!(s.available_input(), 5);
        drop(r);
        let r = s.prepare(&b, &mut out).unwrap().unwrap();
        s.commit(r, &b).unwrap();
        assert_eq!(s.available_input(), 1);
    }
    #[test]
    fn whole_write_waits_for_space_and_failed_delivery_accepts_nothing() {
        let mut s = SerialService::new();
        let mut f = Fake::new();
        f.ready = false;
        let mut out = [0; p::HEADER + p::MAX_INPUT];
        let b = request(p::WRITE, 0, &[b'x'; 4096]);
        let r = s.prepare(&b, &mut out).unwrap().unwrap();
        drop(r);
        assert_eq!(s.queued_output(), 0);
        let r = s.prepare(&b, &mut out).unwrap().unwrap();
        s.commit(r, &b).unwrap();
        assert_eq!(s.queued_output(), 4096);
        let next = request(p::WRITE, 0, b"suffix");
        assert!(s.prepare(&next, &mut out).unwrap().is_none());
        s.pump(&mut f);
        assert!(s.prepare(&next, &mut out).unwrap().is_none());
        f.ready = true;
        s.pump(&mut f);
        let r = s.prepare(&next, &mut out).unwrap().unwrap();
        assert_eq!(p::decode(&out[..r.len]).unwrap().argument, 6);
        s.commit(r, &next).unwrap();
        while s.queued_output() != 0 {
            s.pump(&mut f);
        }
        assert_eq!(&f.tx[..4096], &[b'x'; 4096]);
        assert_eq!(&f.tx[4096..], b"suffix");
    }
    #[test]
    fn failed_overrun_reply_retains_latch_and_surviving_bytes() {
        let mut s = SerialService::new();
        let mut f = Fake::new();
        f.error = 2;
        f.rx.extend(b"safe");
        s.pump(&mut f);
        let b = request(p::READ, 256, &[]);
        let mut out = [0; p::HEADER + p::MAX_INPUT];
        let r = s.prepare(&b, &mut out).unwrap().unwrap();
        assert_eq!(p::decode(&out[..r.len]).unwrap().status, OVERRUN);
        drop(r);
        assert!(s.has_input_error());
        assert_eq!(s.available_input(), 4);
        let r = s.prepare(&b, &mut out).unwrap().unwrap();
        s.commit(r, &b).unwrap();
        assert!(!s.has_input_error());
        assert_eq!(s.available_input(), 4);
        let r = s.prepare(&b, &mut out).unwrap().unwrap();
        assert_eq!(&out[p::HEADER..r.len], b"safe");
        s.commit(r, &b).unwrap();
        assert_eq!(s.available_input(), 0);
    }
    #[test]
    fn malformed_and_unsupported_requests_are_explicit() {
        let s = SerialService::new();
        let mut out = [0; p::HEADER + p::MAX_INPUT];
        assert!(s.prepare(&request(p::READ, 0, &[]), &mut out).is_err());
        assert!(s.prepare(&request(p::WRITE, 0, &[]), &mut out).is_err());
        let r = s
            .prepare(&request(p::COLORS, 0, &[]), &mut out)
            .unwrap()
            .unwrap();
        assert_eq!(p::decode(&out[..r.len]).unwrap().status, -95);
        let r = s
            .prepare(&request(p::INFO, 0, &[]), &mut out)
            .unwrap()
            .unwrap();
        assert_eq!(p::decode(&out[..r.len]).unwrap().value, INPUT | OUTPUT);
    }
}
