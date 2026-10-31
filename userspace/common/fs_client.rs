#![allow(dead_code)]
use super::{fs_protocol as p, rt};
pub struct Client {
    pub handle: usize,
    request: u32,
    pub buffer: [u8; p::MAX_MESSAGE],
    pub length: usize,
    pub flags: u32,
}
impl Client {
    pub fn new(handle: usize) -> Self {
        Self {
            handle,
            request: 0,
            buffer: [0; p::MAX_MESSAGE],
            length: 0,
            flags: 0,
        }
    }
    fn transact(&mut self, op: u16, object: u64, flags: u32, body_len: usize) -> p::Header {
        self.request = self.request.checked_add(1).unwrap_or_else(|| rt::exit(60));
        p::encode(
            p::Header {
                op,
                request: self.request,
                status: 0,
                flags,
                object,
            },
            &mut self.buffer,
        );
        rt::send(self.handle, &self.buffer[..p::HEADER + body_len]);
        let length = rt::receive(self.handle, &mut self.buffer);
        let h = p::decode(&self.buffer[..length]).unwrap_or_else(|| rt::exit(60));
        if h.op != op || h.request != self.request || h.status != 0 || h.flags & !p::EOF != 0 {
            rt::exit(60)
        }
        self.length = length - p::HEADER;
        self.flags = h.flags;
        h
    }
    pub fn open(&mut self, path: &str, kind: u32) -> (u64, usize) {
        if path.len() > 255 {
            rt::exit(60)
        }
        self.buffer[p::HEADER..p::HEADER + path.len()].copy_from_slice(path.as_bytes());
        let h = self.transact(p::OPEN, 0, kind, path.len());
        if self.length != 16
            || h.object == 0
            || h.flags != 0
            || self.buffer[p::HEADER..p::HEADER + 4] != kind.to_le_bytes()
            || self.buffer[p::HEADER + 4..p::HEADER + 8] != [0; 4]
        {
            rt::exit(60)
        }
        let len = usize::try_from(u64::from_le_bytes(
            self.buffer[p::HEADER + 8..p::HEADER + 16]
                .try_into()
                .unwrap(),
        ))
        .unwrap_or_else(|_| rt::exit(60));
        (h.object, len)
    }
    pub fn read(&mut self, id: u64, offset: usize, length: usize) -> usize {
        if length == 0 || length > p::CHUNK {
            rt::exit(60)
        }
        self.buffer[p::HEADER..p::HEADER + 8].copy_from_slice(&(offset as u64).to_le_bytes());
        self.buffer[p::HEADER + 8..p::HEADER + 12].copy_from_slice(&(length as u32).to_le_bytes());
        self.buffer[p::HEADER + 12..p::HEADER + 16].fill(0);
        let h = self.transact(p::READ, id, 0, 16);
        if h.object != id || self.length > length || (self.length < length && h.flags != p::EOF) {
            rt::exit(60)
        }
        self.length
    }
    pub fn close(&mut self, id: u64) {
        let h = self.transact(p::CLOSE, id, 0, 0);
        if h.object != id || self.length != 0 || h.flags != 0 {
            rt::exit(60)
        }
    }
    pub fn stat(&mut self, id: u64) {
        let h = self.transact(p::STAT, id, 0, 0);
        if h.object != id || self.length != 16 || h.flags != 0 {
            rt::exit(60)
        }
    }
    pub fn list(&mut self, id: u64, cursor: u32) -> Option<u32> {
        self.buffer[p::HEADER..p::HEADER + 4].copy_from_slice(&cursor.to_le_bytes());
        self.buffer[p::HEADER + 4..p::HEADER + 8].fill(0);
        let h = self.transact(p::LIST, id, 0, 8);
        if h.object != id {
            rt::exit(60)
        }
        if h.flags == p::EOF {
            if self.length != 0 {
                rt::exit(60)
            }
            return None;
        }
        if self.length <= 16 || self.length > 271 {
            rt::exit(60)
        }
        let next = u32::from_le_bytes(self.buffer[p::HEADER..p::HEADER + 4].try_into().unwrap());
        if next <= cursor {
            rt::exit(60)
        }
        Some(next)
    }
}
