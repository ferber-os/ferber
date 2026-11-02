//! One outstanding console request; the caller can service control channels while waiting.
#![allow(dead_code)]
use super::{console_protocol as p, rt};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Busy,
    Invalid,
    Exhausted,
    System(isize),
    Remote(i32),
}
#[derive(Clone, Copy)]
struct Pending {
    header: p::Header,
    length: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Response {
    pub operation: u16,
    pub length: usize,
    pub value: u64,
}
pub struct Client {
    pub handle: usize,
    request: u32,
    pending: Option<Pending>,
    pub buffer: [u8; p::HEADER + p::MAX_TEXT],
}
impl Client {
    pub const fn new(handle: usize) -> Self {
        Self {
            handle,
            request: 0,
            pending: None,
            buffer: [0; p::HEADER + p::MAX_TEXT],
        }
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    fn begin(&mut self, operation: u16, argument: u32, data: &[u8]) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        if data.len() > p::MAX_TEXT {
            return Err(Error::Invalid);
        }
        let id = self.request.checked_add(1).ok_or(Error::Exhausted)?;
        let header = p::Header {
            operation,
            request: id,
            status: 0,
            argument,
            value: 0,
        };
        let mut bytes = [0; p::HEADER];
        p::encode(header, &mut bytes);
        self.buffer[..p::HEADER].copy_from_slice(&bytes);
        self.buffer[p::HEADER..p::HEADER + data.len()].copy_from_slice(data);
        p::request(&self.buffer[..p::HEADER + data.len()]).map_err(|_| Error::Invalid)?;
        let result = rt::call(
            8,
            [
                self.handle,
                self.buffer.as_ptr() as usize,
                p::HEADER + data.len(),
                0,
                0,
                0,
            ],
        );
        if result != 0 {
            return Err(Error::System(result));
        }
        self.request = id;
        self.pending = Some(Pending {
            header,
            length: data.len(),
        });
        Ok(())
    }
    pub fn read(&mut self, maximum: usize) -> Result<(), Error> {
        let maximum = u32::try_from(maximum).map_err(|_| Error::Invalid)?;
        self.begin(p::READ, maximum, &[])
    }
    pub fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        self.begin(p::WRITE, 0, data)
    }
    pub fn info(&mut self) -> Result<(), Error> {
        self.begin(p::INFO, 0, &[])
    }
    /// Queue-full sends leave no request in flight; callers may yield and retry the same data.
    /// A receive error leaves the outstanding request intact until the caller exits or retries.
    pub fn receive(&mut self) -> Result<Response, Error> {
        let pending = self.pending.ok_or(Error::Invalid)?;
        let mut counts = [0u32; 2];
        let result = rt::call(
            9,
            [
                self.handle,
                self.buffer.as_mut_ptr() as usize,
                self.buffer.len(),
                0,
                0,
                counts.as_mut_ptr() as usize,
            ],
        );
        if result != 0 {
            return Err(Error::System(result));
        }
        if counts[1] != 0 || counts[0] as usize > self.buffer.len() {
            return Err(Error::Invalid);
        }
        let response = validate(pending, &self.buffer[..counts[0] as usize])?;
        self.pending = None;
        if response.0.status != 0 {
            return Err(Error::Remote(response.0.status));
        }
        Ok(Response {
            operation: response.0.operation,
            length: response.1,
            value: response.0.value,
        })
    }
}
fn validate(pending: Pending, bytes: &[u8]) -> Result<(p::Header, usize), Error> {
    let h = p::decode(bytes).map_err(|_| Error::Invalid)?;
    if h.operation != pending.header.operation
        || h.request != pending.header.request
        || h.status > 0
    {
        return Err(Error::Invalid);
    }
    let length = bytes.len() - p::HEADER;
    if h.status < 0 {
        if length != 0 || h.argument != 0 || h.value != 0 {
            return Err(Error::Invalid);
        }
        return Ok((h, 0));
    }
    let valid = match h.operation {
        p::READ => {
            length > 0
                && length <= pending.header.argument as usize
                && h.argument as usize == length
                && h.value == 0
        }
        p::WRITE => length == 0 && h.argument as usize == pending.length && h.value == 0,
        p::INFO => length == 0 && h.argument == 0 && h.value & !7 == 0,
        _ => false,
    };
    if !valid {
        return Err(Error::Invalid);
    }
    Ok((h, length))
}
