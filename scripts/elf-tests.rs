//! Host tests exercise the same allocation-free parser used by the kernel.
#[path = "../ferber/src/kernel/elf.rs"]
mod elf;

use elf::{ElfError, ElfImage};

const USER_MAX: usize = 1 << 38;
const MACHINES: [u16; 2] = [62, 243];

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn image(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x2100];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put16(&mut bytes, 16, 2);
    put16(&mut bytes, 18, machine);
    put32(&mut bytes, 20, 1);
    put64(&mut bytes, 24, 0x10000);
    put64(&mut bytes, 32, 64);
    put16(&mut bytes, 52, 64);
    put16(&mut bytes, 54, 56);
    put16(&mut bytes, 56, 2);
    for (index, flags, offset, vaddr, memsz) in [
        (0, 5, 0x1000, 0x10000, 0x1000),
        (1, 6, 0x2000, 0x12000, 0x2000),
    ] {
        let ph = 64 + index * 56;
        put32(&mut bytes, ph, 1);
        put32(&mut bytes, ph + 4, flags);
        put64(&mut bytes, ph + 8, offset);
        put64(&mut bytes, ph + 16, vaddr);
        put64(&mut bytes, ph + 32, 8);
        put64(&mut bytes, ph + 40, memsz);
        put64(&mut bytes, ph + 48, 4096);
    }
    bytes[0x1000..0x1008].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    bytes[0x2000..0x2008].copy_from_slice(&[8, 7, 6, 5, 4, 3, 2, 1]);
    bytes
}

fn reject(bytes: &[u8], machine: u16, expected: ElfError) {
    assert_eq!(
        ElfImage::parse(bytes, machine, USER_MAX).unwrap_err(),
        expected
    );
}

#[test]
fn static_segments_preserve_bytes_bss_and_permissions() {
    for machine in MACHINES {
        let bytes = image(machine);
        let elf = ElfImage::parse(&bytes, machine, USER_MAX).unwrap();
        assert_eq!(elf.entry(), 0x10000);
        let segments: Vec<_> = elf.segments().collect();
        assert_eq!(segments.len(), 2);
        let text = segments[0];
        assert_eq!(text.data, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(text.virtual_address, 0x10000);
        assert_eq!(text.page_start, 0x10000);
        assert_eq!(text.page_size, 4096);
        assert_eq!(text.memory_size, 4096);
        assert!(text.readable && text.executable && !text.writable);
        let data = segments[1];
        assert_eq!(data.data.len(), 8);
        assert_eq!(data.memory_size, 8192);
        assert_eq!(data.page_size, 8192);
        assert!(data.readable && data.writable && !data.executable);
    }
}

#[test]
fn identification_type_versions_and_machine_are_checked() {
    for machine in MACHINES {
        for (offset, value) in [(0, 0), (4, 1), (5, 2), (6, 2), (7, 3), (8, 1)] {
            let mut bytes = image(machine);
            bytes[offset] = value;
            reject(&bytes, machine, ElfError::Header);
        }
        for (offset, value) in [(16, 3), (18, 0), (20, 2), (52, 63), (54, 55)] {
            let mut bytes = image(machine);
            put16(&mut bytes, offset, value);
            reject(&bytes, machine, ElfError::Header);
        }
        let bytes = image(machine);
        reject(
            &bytes,
            if machine == 62 { 243 } else { 62 },
            ElfError::Header,
        );
        reject(&bytes, 0, ElfError::Header);
    }
}

#[test]
fn header_program_table_and_payload_truncation_are_checked() {
    for machine in MACHINES {
        let bytes = image(machine);
        for end in 0..64 {
            reject(&bytes[..end], machine, ElfError::Header);
        }
        for end in 64..176 {
            reject(&bytes[..end], machine, ElfError::Bounds);
        }
        for end in [0x1000, 0x1007, 0x2000, 0x2007] {
            reject(&bytes[..end], machine, ElfError::Bounds);
        }
        for offset in [0u64, 63] {
            let mut bytes = image(machine);
            put64(&mut bytes, 32, offset);
            reject(&bytes, machine, ElfError::Header);
        }
        let mut bytes = image(machine);
        put64(&mut bytes, 32, u64::MAX - 1);
        reject(&bytes, machine, ElfError::Bounds);
    }
}

#[test]
fn all_load_permissions_are_explicit_and_wx_is_rejected() {
    for machine in MACHINES {
        for flags in [0, 1, 2, 3, 7, 8, 0x8000_0004] {
            let mut bytes = image(machine);
            put32(&mut bytes, 120 + 4, flags);
            reject(&bytes, machine, ElfError::Permissions);
        }
        let mut bytes = image(machine);
        put32(&mut bytes, 120 + 4, 4);
        assert!(ElfImage::parse(&bytes, machine, USER_MAX).is_ok());
    }
}

#[test]
fn dynamic_interpreter_tls_unknown_and_executable_stack_are_rejected() {
    for machine in MACHINES {
        for kind in [2, 3, 5, 7, 0x6474_e552, u32::MAX, 0x6474_e551] {
            let mut bytes = image(machine);
            put32(&mut bytes, 64, kind);
            reject(&bytes, machine, ElfError::Unsupported);
        }
        for kind in [0, 4, 6, 0x6474_e551] {
            let mut bytes = image(machine);
            put32(&mut bytes, 120, kind);
            assert!(ElfImage::parse(&bytes, machine, USER_MAX).is_ok());
        }
    }
}

#[test]
fn bounds_overflow_null_and_upper_user_range_are_rejected() {
    for machine in MACHINES {
        for (field, value) in [
            (8, u64::MAX - 4095),
            (16, 0),
            (16, USER_MAX as u64),
            (16, u64::MAX - 4095),
            (32, 4097),
            (40, 0),
            (40, u64::MAX),
        ] {
            let mut bytes = image(machine);
            put64(&mut bytes, 64 + field, value);
            reject(&bytes, machine, ElfError::Bounds);
        }
        let mut bytes = image(machine);
        put64(&mut bytes, 120 + 16, USER_MAX as u64 - 4096);
        reject(&bytes, machine, ElfError::Bounds);
    }
}

#[test]
fn architecture_user_ceiling_is_exclusive_and_page_rounded() {
    for (machine, user_max) in [(62, 1usize << 47), (243, 1usize << 38)] {
        let mut bytes = image(machine);
        put64(&mut bytes, 120 + 16, user_max as u64 - 8192);
        assert!(ElfImage::parse(&bytes, machine, user_max).is_ok());
        put64(&mut bytes, 120 + 40, 8193);
        assert_eq!(
            ElfImage::parse(&bytes, machine, user_max).unwrap_err(),
            ElfError::Bounds
        );
        put64(&mut bytes, 120 + 16, 0xffff_ffff_ffff_f000);
        assert_eq!(
            ElfImage::parse(&bytes, machine, user_max).unwrap_err(),
            ElfError::Bounds
        );
    }
}

#[test]
fn alignment_and_page_congruence_are_required() {
    for machine in MACHINES {
        for (field, value) in [(48, 3), (48, 8192), (8, 0x1001), (16, 0x10001)] {
            let mut bytes = image(machine);
            put64(&mut bytes, 64 + field, value);
            reject(&bytes, machine, ElfError::Alignment);
        }
        for align in [0, 1, 4096] {
            let mut bytes = image(machine);
            put64(&mut bytes, 64 + 48, align);
            assert!(ElfImage::parse(&bytes, machine, USER_MAX).is_ok());
        }
        let mut bytes = image(machine);
        put64(&mut bytes, 64 + 8, 0x1002);
        put64(&mut bytes, 64 + 16, 0x10002);
        put64(&mut bytes, 24, 0x10002);
        let elf = ElfImage::parse(&bytes, machine, USER_MAX).unwrap();
        let text = elf.segments().next().unwrap();
        assert_eq!(text.virtual_address - text.page_start, 2);
        assert_eq!(text.page_size, 8192);
    }
}

#[test]
fn byte_disjoint_segments_cannot_share_a_mapping_page() {
    for machine in MACHINES {
        let mut bytes = image(machine);
        put64(&mut bytes, 64 + 40, 8);
        put64(&mut bytes, 120 + 8, 0x2010);
        put64(&mut bytes, 120 + 16, 0x10010);
        put64(&mut bytes, 120 + 40, 8);
        reject(&bytes, machine, ElfError::Overlap);
        let mut bytes = image(machine);
        put64(&mut bytes, 120 + 16, 0x10000);
        reject(&bytes, machine, ElfError::Overlap);
    }
}

#[test]
fn entry_must_be_file_backed_executable_and_riscv_aligned() {
    for machine in MACHINES {
        for entry in [0, 0x10008, 0x11000, 0x12000, USER_MAX as u64] {
            let mut bytes = image(machine);
            put64(&mut bytes, 24, entry);
            reject(&bytes, machine, ElfError::Entry);
        }
        let mut bytes = image(machine);
        put64(&mut bytes, 24, 0x10001);
        if machine == 243 {
            reject(&bytes, machine, ElfError::Entry);
        } else {
            assert!(ElfImage::parse(&bytes, machine, USER_MAX).is_ok());
        }
        put16(&mut bytes, 56, 1);
        put32(&mut bytes, 64, 0);
        reject(&bytes, machine, ElfError::Entry);
    }
}

#[test]
fn unsupported_cpu_abi_flags_are_rejected() {
    for (machine, flags) in [(62, 1), (243, 2), (243, 4), (243, 8), (243, 16)] {
        let mut bytes = image(machine);
        put32(&mut bytes, 48, flags);
        reject(&bytes, machine, ElfError::Unsupported);
    }
    for flags in [0, 1, 5] {
        let mut bytes = image(243);
        put32(&mut bytes, 48, flags);
        assert!(ElfImage::parse(&bytes, 243, USER_MAX).is_ok());
    }
}

#[test]
fn riscv_attributes_are_ignored_only_after_file_bounds_validation() {
    let mut bytes = image(243);
    put32(&mut bytes, 120, 0x7000_0003);
    assert!(ElfImage::parse(&bytes, 243, USER_MAX).is_ok());
    put64(&mut bytes, 120 + 8, u64::MAX);
    reject(&bytes, 243, ElfError::Bounds);
    let mut bytes = image(62);
    put32(&mut bytes, 120, 0x7000_0003);
    reject(&bytes, 62, ElfError::Unsupported);
}

#[test]
fn resources_are_bounded_before_mapping() {
    for machine in MACHINES {
        let mut bytes = image(machine);
        bytes.resize(elf::MAX_IMAGE_SIZE + 1, 0);
        reject(&bytes, machine, ElfError::Limit);
        let mut bytes = image(machine);
        put16(&mut bytes, 56, 33);
        reject(&bytes, machine, ElfError::Limit);
        let mut bytes = image(machine);
        put64(&mut bytes, 120 + 40, elf::MAX_MAPPED_SIZE as u64);
        reject(&bytes, machine, ElfError::Limit);
        let mut bytes = image(machine);
        put16(&mut bytes, 56, 17);
        for index in 0..17 {
            let ph = 64 + index * 56;
            put32(&mut bytes, ph, 1);
            put32(&mut bytes, ph + 4, 5);
            put64(&mut bytes, ph + 8, 0x1000);
            put64(&mut bytes, ph + 16, 0x10000 + index as u64 * 4096);
            put64(&mut bytes, ph + 32, 8);
            put64(&mut bytes, ph + 40, 4096);
            put64(&mut bytes, ph + 48, 4096);
        }
        reject(&bytes, machine, ElfError::Limit);
    }
}

fn add_section(bytes: &mut [u8], kind: u32) {
    put64(bytes, 40, 0x1800);
    put16(bytes, 58, 64);
    put16(bytes, 60, 2);
    put32(bytes, 0x1840 + 4, kind);
    put64(bytes, 0x1840 + 24, 0x2000);
    put64(bytes, 0x1840 + 32, 8);
}

#[test]
fn section_table_is_bounded_and_relocations_and_tls_are_rejected() {
    for machine in MACHINES {
        for kind in [4, 9, 19] {
            let mut bytes = image(machine);
            add_section(&mut bytes, kind);
            reject(&bytes, machine, ElfError::Unsupported);
        }
        let mut bytes = image(machine);
        add_section(&mut bytes, 1);
        assert!(ElfImage::parse(&bytes, machine, USER_MAX).is_ok());
        put64(&mut bytes, 0x1840 + 8, 0x400);
        reject(&bytes, machine, ElfError::Unsupported);
        let mut bytes = image(machine);
        add_section(&mut bytes, 1);
        put64(&mut bytes, 0x1840 + 24, u64::MAX);
        reject(&bytes, machine, ElfError::Bounds);
        let mut bytes = image(machine);
        add_section(&mut bytes, 8);
        put64(&mut bytes, 0x1840 + 24, u64::MAX);
        assert!(ElfImage::parse(&bytes, machine, USER_MAX).is_ok());
        let mut bytes = image(machine);
        put16(&mut bytes, 60, 257);
        reject(&bytes, machine, ElfError::Limit);
        let mut bytes = image(machine);
        add_section(&mut bytes, 1);
        let truncated_table = bytes.len() as u64 - 64;
        put64(&mut bytes, 40, truncated_table);
        reject(&bytes, machine, ElfError::Bounds);
        let mut bytes = image(machine);
        put64(&mut bytes, 40, 0x1800);
        reject(&bytes, machine, ElfError::Header);
        for (field, value) in [(58, 63), (60, 0), (62, 2), (62, 0xffff)] {
            let mut bytes = image(machine);
            add_section(&mut bytes, 1);
            put16(&mut bytes, field, value);
            reject(&bytes, machine, ElfError::Header);
        }
    }
}

#[test]
fn arbitrary_single_byte_mutations_and_truncations_never_panic() {
    for machine in MACHINES {
        let mut bytes = image(machine);
        for offset in 0..176 {
            let original = bytes[offset];
            for value in [0, 1, 0x7f, 0xff] {
                bytes[offset] = value;
                let _ = ElfImage::parse(&bytes, machine, USER_MAX);
            }
            bytes[offset] = original;
        }
        for end in 0..bytes.len() {
            let _ = ElfImage::parse(&bytes[..end], machine, USER_MAX);
        }
    }
}
