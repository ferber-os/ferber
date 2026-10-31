//! Bounded bitmap rendering over an exclusively owned framebuffer mapping.
#![allow(dead_code)]
pub const CELL_WIDTH: usize = 12;
pub const CELL_HEIGHT: usize = 24;
const FONT: &[u8; 4560] = include_bytes!("../userspace/console/font/ascii.bin");
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb,
    Bgr,
}
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidSurface;
pub struct Terminal<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
    stride: usize,
    columns: usize,
    rows: usize,
    column: usize,
    row: usize,
    foreground: u32,
    background: u32,
    format: PixelFormat,
}
impl<'a> Terminal<'a> {
    pub fn new(
        pixels: &'a mut [u32],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
    ) -> Result<Self, InvalidSurface> {
        if width < CELL_WIDTH
            || height < CELL_HEIGHT
            || stride < width
            || stride
                .checked_mul(height)
                .is_none_or(|size| size > pixels.len())
        {
            return Err(InvalidSurface);
        }
        Ok(Self {
            pixels,
            width,
            height,
            stride,
            columns: width / CELL_WIDTH,
            rows: height / CELL_HEIGHT,
            column: 0,
            row: 0,
            foreground: 0xd8dee9,
            background: 0x101216,
            format,
        })
    }
    pub fn colors(&mut self, foreground: u32, background: u32) -> Result<(), InvalidSurface> {
        if foreground > 0xffffff || background > 0xffffff {
            return Err(InvalidSurface);
        }
        self.foreground = foreground;
        self.background = background;
        Ok(())
    }
    fn packed(&self, rgb: u32) -> u32 {
        match self.format {
            PixelFormat::Bgr => rgb,
            PixelFormat::Rgb => ((rgb & 255) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 255),
        }
    }
    fn store(&mut self, x: usize, y: usize, rgb: u32) {
        let value = self.packed(rgb);
        // SAFETY: constructor checked stride*height; all callers use visible coordinates.
        unsafe {
            self.pixels
                .as_mut_ptr()
                .add(y * self.stride + x)
                .write_volatile(value);
        }
    }
    pub fn clear(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.store(x, y, self.background);
            }
        }
        self.column = 0;
        self.row = 0;
    }
    fn newline(&mut self) {
        self.column = 0;
        if self.row + 1 < self.rows {
            self.row += 1;
            return;
        }
        let height = self.rows * CELL_HEIGHT;
        for y in 0..height - CELL_HEIGHT {
            for x in 0..self.width {
                // SAFETY: both rows lie inside the constructor-validated visible surface.
                let value = unsafe {
                    self.pixels
                        .as_ptr()
                        .add((y + CELL_HEIGHT) * self.stride + x)
                        .read_volatile()
                };
                unsafe {
                    self.pixels
                        .as_mut_ptr()
                        .add(y * self.stride + x)
                        .write_volatile(value);
                }
            }
        }
        for y in height - CELL_HEIGHT..height {
            for x in 0..self.width {
                self.store(x, y, self.background);
            }
        }
    }
    pub fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.byte(byte);
        }
    }
    fn byte(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.newline();
                return;
            }
            b'\r' => {
                self.column = 0;
                return;
            }
            8 => {
                if self.column > 0 {
                    self.column -= 1;
                    self.glyph(b' ');
                }
                return;
            }
            b'\t' => {
                let count = 4 - self.column % 4;
                for _ in 0..count {
                    self.byte(b' ');
                }
                return;
            }
            _ => {}
        }
        if self.column == self.columns {
            self.newline();
        }
        self.glyph(if byte.is_ascii_graphic() || byte == b' ' {
            byte
        } else {
            b'?'
        });
        self.column += 1;
    }
    fn glyph(&mut self, byte: u8) {
        let offset = (byte as usize - 32) * 48;
        for y in 0..CELL_HEIGHT {
            let bits = u16::from_be_bytes([FONT[offset + y * 2], FONT[offset + y * 2 + 1]]);
            for x in 0..CELL_WIDTH {
                let color = if bits & (0x8000 >> x) != 0 {
                    self.foreground
                } else {
                    self.background
                };
                self.store(
                    self.column * CELL_WIDTH + x,
                    self.row * CELL_HEIGHT + y,
                    color,
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_padding() {
        let mut pixels = vec![0xdeadbeef; 30 * 49];
        let mut t = Terminal::new(&mut pixels, 24, 49, 30, PixelFormat::Bgr).unwrap();
        t.clear();
        t.write(b"abcdef\nmore\nlines\t?\x01");
        for y in 0..49 {
            assert!(pixels[y * 30 + 24..y * 30 + 30]
                .iter()
                .all(|p| *p == 0xdeadbeef));
        }
        assert!(Terminal::new(&mut pixels, 31, 49, 30, PixelFormat::Bgr).is_err());
        assert!(Terminal::new(&mut pixels, 24, usize::MAX, 30, PixelFormat::Bgr).is_err());
    }
    #[test]
    fn colors_scroll_and_backspace() {
        let mut pixels = vec![0; 12 * 48];
        let mut t = Terminal::new(&mut pixels, 12, 48, 12, PixelFormat::Rgb).unwrap();
        t.colors(0xff0000, 0x123456).unwrap();
        t.clear();
        t.write(b"A\nB\n");
        assert!(t.pixels[..12 * 24].contains(&0xff));
        assert!(t.pixels[12 * 24..].iter().all(|p| *p == 0x563412));
        t.write(b"Z\x08");
        assert!(t.pixels[12 * 24..].iter().all(|p| *p == 0x563412));
        assert!(t.colors(0x1000000, 0).is_err());
    }
}
