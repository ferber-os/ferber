//! Test-only ELF that exits immediately, proving an external image was executed.
use std::{env, fs};

fn main() {
    let args: Vec<_> = env::args().collect();
    assert_eq!(args.len(), 3, "usage: fixture TARGET OUTPUT");
    let (machine, code): (u16, &[u8]) = match args[1].as_str() {
        "riscv64gc-unknown-none-elf" => (243, &[
            0x13, 0x05, 0x10, 0x00, // li a0, 1
            0x93, 0x08, 0x10, 0x00, // li a7, 1
            0x73, 0x00, 0x00, 0x00, // ecall
        ]),
        "x86_64-unknown-none" => (62, &[
            0xbf, 1, 0, 0, 0, // mov edi, 1
            0xb8, 1, 0, 0, 0, // mov eax, 1
            0x0f, 0x05, // syscall
        ]),
        _ => panic!("unsupported fixture target"),
    };
    let mut image = vec![0; 256 + code.len()];
    image[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    let u16_at = |bytes: &mut [u8], at, value: u16| {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    };
    let u32_at = |bytes: &mut [u8], at, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    let u64_at = |bytes: &mut [u8], at, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    u16_at(&mut image, 16, 2);
    u16_at(&mut image, 18, machine);
    u32_at(&mut image, 20, 1);
    u64_at(&mut image, 24, 0x400100);
    u64_at(&mut image, 32, 64);
    u16_at(&mut image, 52, 64);
    u16_at(&mut image, 54, 56);
    u16_at(&mut image, 56, 1);
    u32_at(&mut image, 64, 1);
    u32_at(&mut image, 68, 5);
    u64_at(&mut image, 80, 0x400000);
    let size = image.len() as u64;
    u64_at(&mut image, 96, size);
    u64_at(&mut image, 104, size);
    u64_at(&mut image, 112, 4096);
    image[256..].copy_from_slice(code);
    fs::write(&args[2], image).expect("write test payload");
}
