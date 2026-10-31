//! Console sessions carry bounded text and input, never implicit device authority.
#![allow(dead_code)]
pub const HEADER: usize = 32;
pub const MAX_TEXT: usize = 4096;
pub const MAX_INPUT: usize = 256;
pub const READ: u16 = 1;
pub const WRITE: u16 = 2;
pub const INFO: u16 = 3;
pub const COLORS: u16 = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub operation: u16,
    pub request: u32,
    pub status: i32,
    pub argument: u32,
    pub value: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolError;
pub fn decode(bytes: &[u8]) -> Result<Header, ProtocolError> {
    if bytes.len() < HEADER
        || bytes.len() > HEADER + MAX_TEXT
        || &bytes[..8] != b"FRCONS01"
        || bytes[8..10] != 1u16.to_le_bytes()
    {
        return Err(ProtocolError);
    }
    let h = Header {
        operation: u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
        request: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        status: i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
        argument: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        value: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
    };
    if !matches!(h.operation, READ | WRITE | INFO | COLORS) || h.request == 0 {
        return Err(ProtocolError);
    }
    Ok(h)
}
pub fn request(bytes: &[u8]) -> Result<Header, ProtocolError> {
    let h = decode(bytes)?;
    if h.status != 0 {
        return Err(ProtocolError);
    }
    let len = bytes.len() - HEADER;
    let valid = match h.operation {
        READ => len == 0 && h.value == 0 && (1..=MAX_INPUT as u32).contains(&h.argument),
        WRITE => len > 0 && h.argument == 0 && h.value == 0,
        INFO => len == 0 && h.argument == 0 && h.value == 0,
        COLORS => len == 0 && h.argument <= 0xffffff && h.value <= 0xffffff,
        _ => false,
    };
    if valid {
        Ok(h)
    } else {
        Err(ProtocolError)
    }
}
pub fn encode(h: Header, out: &mut [u8; HEADER]) {
    out.fill(0);
    out[..8].copy_from_slice(b"FRCONS01");
    out[8..10].copy_from_slice(&1u16.to_le_bytes());
    out[10..12].copy_from_slice(&h.operation.to_le_bytes());
    out[12..16].copy_from_slice(&h.request.to_le_bytes());
    out[16..20].copy_from_slice(&h.status.to_le_bytes());
    out[20..24].copy_from_slice(&h.argument.to_le_bytes());
    out[24..].copy_from_slice(&h.value.to_le_bytes());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_requests_reject_missing_and_extra_fields() {
        for op in [READ, INFO, COLORS] {
            let h = Header {
                operation: op,
                request: 1,
                status: 0,
                argument: if op == READ { 256 } else { 0 },
                value: 0,
            };
            let mut b = [0; HEADER];
            encode(h, &mut b);
            assert_eq!(request(&b), Ok(h));
            for n in 0..HEADER {
                assert!(request(&b[..n]).is_err());
            }
            let mut x = b;
            x[16] = 1;
            assert!(request(&x).is_err());
        }
        let mut b = [0; HEADER];
        encode(
            Header {
                operation: READ,
                request: 1,
                status: 0,
                argument: 257,
                value: 0,
            },
            &mut b,
        );
        assert!(request(&b).is_err());
        encode(
            Header {
                operation: COLORS,
                request: 1,
                status: 0,
                argument: 0,
                value: 1 << 24,
            },
            &mut b,
        );
        assert!(request(&b).is_err());
    }
}
