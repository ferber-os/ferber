//! PCI configuration MMIO requires accumulator MOVs (AMD 56713 §2.1.5.1).
#![allow(dead_code)]
fn address(base: usize, length: usize, offset: usize, width: usize) -> Option<usize> {
    let address = base.checked_add(offset)?;
    if address & (width - 1) != 0 || offset.checked_add(width)? > length {
        return None;
    }
    address.checked_add(width - 1)?;
    Some(address)
}
/// # Safety
/// The range is live, supervisor-owned UC PCI configuration memory.
#[inline(always)]
pub unsafe fn read32(base: usize, length: usize, offset: usize) -> Option<u32> {
    let address = address(base, length, offset, 4)?;
    let value: u32;
    core::arch::asm!("mov eax, dword ptr [{address}]", address = in(reg) address,
        out("eax") value, options(nostack, preserves_flags));
    Some(value)
}
/// # Safety
/// The range is live UC PCI configuration memory; caller owns this register write.
#[inline(always)]
pub unsafe fn write32(base: usize, length: usize, offset: usize, value: u32) -> Option<()> {
    let address = address(base, length, offset, 4)?;
    core::arch::asm!("mov dword ptr [{address}], eax", address = in(reg) address,
        in("eax") value, options(nostack, preserves_flags));
    Some(())
}
/// # Safety
/// The range is live UC PCI configuration memory; caller owns this register write.
#[inline(always)]
pub unsafe fn write16(base: usize, length: usize, offset: usize, value: u16) -> Option<()> {
    let address = address(base, length, offset, 2)?;
    core::arch::asm!("mov word ptr [{address}], ax", address = in(reg) address,
        in("ax") value, options(nostack, preserves_flags));
    Some(())
}
