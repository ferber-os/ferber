//! Read-only source ownership survives all process address-space changes.
use super::memory::vmo::Vmo;
use super::sync::SpinLock;
use crate::boot_info::{BootInfo, BootPayload};

static SOURCE: SpinLock<Option<(BootPayload, Vmo)>> = SpinLock::new(None);

/// Publish only after the permanent supervisor RO/NX alias is active.
pub fn retain(info: &BootInfo) {
    let Some((start, end)) = info.boot_payload.bounds().expect("invalid boot payload") else {
        return;
    };
    use super::memory::{
        paging::{AddressSpace, MapFlags},
        phys_map,
    };
    let space = crate::arch::ArchAddressSpace::new().expect("boot payload mapping check");
    for physical in (start..end).step_by(4096) {
        let alias = crate::boot_info::payload::PAYLOAD_BASE + physical - start;
        assert_eq!(
            space.query(alias as usize),
            Some((physical as usize, MapFlags::kernel_ro())),
            "invalid permanent boot payload mapping"
        );
        assert!(
            phys_map::phys_to_virt(physical as usize).is_none(),
            "writable boot payload RAM alias"
        );
        assert!(space.query(physical as usize).is_none(), "low boot payload alias");
    }
    drop(space);
    let backing =
        Vmo::create_physical(start as usize, (end - start) as usize).expect("boot payload backing");
    let mut source = SOURCE.lock();
    assert!(source.is_none(), "boot payload published twice");
    // Boot-lifetime reservation is intentional; physical VMOs never free firmware pages.
    *source = Some((info.boot_payload, backing));
}

#[cfg(any(feature = "userspace-elf-probe", feature = "minimal-init", feature = "userspace-vmo-probe", feature = "userspace-vmo-map-probe", feature = "userspace-vmo-seal-probe", feature = "userspace-job-accounting-probe"))]
pub fn bytes() -> Option<&'static [u8]> {
    let source = SOURCE.lock();
    let (payload, _) = source.as_ref()?;
    // SAFETY: validated permanent read-only alias, retained backing, boot-lifetime reservation.
    Some(unsafe {
        core::slice::from_raw_parts(
            payload.virtual_start as *const u8,
            payload.byte_len as usize,
        )
    })
}
