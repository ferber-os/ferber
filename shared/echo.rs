//! Bounded line echo. It does not interpret commands or own device authority.
#![allow(dead_code)]
pub const PROMPT: &[u8] = b"echo> ";
pub const MAX_LINE: usize = 256;
pub const MAX_OUTPUT: usize = MAX_LINE + 4 + PROMPT.len();
pub struct Echo {
    line: [u8; MAX_LINE],
    length: usize,
    after_cr: bool,
}
impl Echo {
    pub const fn new() -> Self {
        Self {
            line: [0; MAX_LINE],
            length: 0,
            after_cr: false,
        }
    }
    /// One input byte produces at most one bounded reply; the caller delivers it before continuing.
    pub fn input(&mut self, byte: u8, output: &mut [u8; MAX_OUTPUT]) -> usize {
        if byte == b'\n' && self.after_cr {
            self.after_cr = false;
            return 0;
        }
        self.after_cr = byte == b'\r';
        match byte {
            b'\r' | b'\n' => {
                output[..2].copy_from_slice(b"\r\n");
                output[2..2 + self.length].copy_from_slice(&self.line[..self.length]);
                output[2 + self.length..4 + self.length].copy_from_slice(b"\r\n");
                output[4 + self.length..4 + self.length + PROMPT.len()].copy_from_slice(PROMPT);
                let count = 4 + self.length + PROMPT.len();
                self.length = 0;
                count
            }
            8 | 127 if self.length != 0 => {
                self.length -= 1;
                output[..3].copy_from_slice(b"\x08 \x08");
                3
            }
            32..=126 if self.length < MAX_LINE => {
                self.line[self.length] = byte;
                self.length += 1;
                output[0] = byte;
                1
            }
            _ => 0,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_controls_and_crlf_keep_one_bounded_line() {
        let mut echo = Echo::new();
        let mut out = [0; MAX_OUTPUT];
        for byte in b"abc" {
            assert_eq!(echo.input(*byte, &mut out), 1);
        }
        assert_eq!(echo.input(127, &mut out), 3);
        assert_eq!(&out[..3], b"\x08 \x08");
        assert_eq!(echo.input(27, &mut out), 0);
        let n = echo.input(b'\r', &mut out);
        assert_eq!(&out[..n], b"\r\nab\r\necho> ");
        assert_eq!(echo.input(b'\n', &mut out), 0);
        assert_eq!(echo.input(8, &mut out), 0);
        let n = echo.input(b'\n', &mut out);
        assert_eq!(&out[..n], b"\r\n\r\necho> ");
    }
    #[test]
    fn a_full_line_cannot_overrun_or_accept_hidden_suffixes() {
        let mut echo = Echo::new();
        let mut out = [0; MAX_OUTPUT];
        for _ in 0..MAX_LINE {
            assert_eq!(echo.input(b'x', &mut out), 1);
        }
        assert_eq!(echo.input(b'y', &mut out), 0);
        let n = echo.input(b'\n', &mut out);
        assert_eq!(n, MAX_OUTPUT);
        assert!(out[2..MAX_LINE + 2].iter().all(|b| *b == b'x'));
        assert_eq!(&out[MAX_LINE + 4..], PROMPT);
    }
}
