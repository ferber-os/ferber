#[path = "../shared/pci_config_access.rs"]
mod access;

#[unsafe(no_mangle)]
pub unsafe fn pci_read_probe(base: usize, offset: usize) -> Option<u32> {
    access::read32(base, 4096, offset)
}
#[unsafe(no_mangle)]
pub unsafe fn pci_write_probe(base: usize, offset: usize, value: u32) -> Option<()> {
    access::write32(base, 4096, offset, value)
}
#[unsafe(no_mangle)]
pub unsafe fn pci_word_probe(base: usize, offset: usize, value: u16) -> Option<()> {
    access::write16(base, 4096, offset, value)
}
#[test]
fn widths_preserve_adjacent_bytes_and_reject_bad_ranges() {
    let mut memory = [0xa5a5a5a5u32; 1024];
    let base = memory.as_mut_ptr() as usize;
    unsafe {
        assert_eq!(access::write32(base, 4096, 0x100, 0x14d81022), Some(()));
        assert_eq!(access::read32(base, 4096, 0x100), Some(0x14d81022));
        assert_eq!(access::write16(base, 4096, 0x102, 0x1234), Some(()));
        assert_eq!(access::read32(base, 4096, 0x100), Some(0x12341022));
        assert_eq!(memory[0x100 / 4 - 1], 0xa5a5a5a5);
        assert_eq!(memory[0x100 / 4 + 1], 0xa5a5a5a5);
        assert_eq!(access::write16(base, 4096, 4094, 0xbeef), Some(()));
        assert_eq!(access::read32(base, 4096, 4092), Some(0xbeefa5a5));
        for offset in [1, 4094, 4096, usize::MAX] {
            assert_eq!(access::read32(base, 4096, offset), None);
            assert_eq!(access::write32(base, 4096, offset, 0), None);
        }
        for offset in [1, 4095, 4096, usize::MAX] {
            assert_eq!(access::write16(base, 4096, offset, 0), None);
        }
        assert_eq!(access::read32(usize::MAX - 1, 4096, 4), None);
        assert_eq!(access::read32(base, 3, 0), None);
    }
}
