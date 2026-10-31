//! Bounds validation shared by platform discovery and host regressions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error;
/// Round a device aperture outward, rejecting physical-width, overflow and all
/// RAM/protected overlaps. The caller separately proves ACPI/PCI provenance.
pub fn validate(
    base: u64,
    length: u64,
    bits: u32,
    forbidden: &[(u64, u64)],
) -> Result<(u64, u64), Error> {
    if base == 0 || length == 0 || !(32..=52).contains(&bits) {
        return Err(Error);
    }
    let start = base & !4095;
    let end = base
        .checked_add(length)
        .and_then(|x| x.checked_add(4095))
        .ok_or(Error)?
        & !4095;
    if end > 1u64 << bits
        || forbidden
            .iter()
            .any(|&(a, b)| a >= b || start < b && a < end)
    {
        return Err(Error);
    }
    Ok((start, end))
}
