//! Resident user copies through supervisor physical aliases.

use core::ptr;

use super::{paging::AddressSpace, phys_map};

const PAGE_SIZE: usize = 4096;
#[cfg(target_arch = "riscv64")]
const USER_MAX: usize = 0x3f_ffff_ffff;
#[cfg(target_arch = "x86_64")]
const USER_MAX: usize = 0x7fff_ffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserMemError {
    InvalidRange,
    NotMapped,
    PermissionDenied,
}

pub(crate) fn preflight(
    space: &impl AddressSpace,
    start: usize,
    len: usize,
    write: bool,
) -> Result<(), UserMemError> {
    if len == 0 {
        return Ok(());
    }
    let last = start
        .checked_add(len - 1)
        .ok_or(UserMemError::InvalidRange)?;
    if start == 0 || start > USER_MAX || last > USER_MAX {
        return Err(UserMemError::InvalidRange);
    }
    let mut address = start;
    loop {
        let (physical, flags) = space.query(address).ok_or(UserMemError::NotMapped)?;
        phys_map::phys_to_virt(physical).ok_or(UserMemError::NotMapped)?;
        let permitted = if write {
            flags.writable
        } else {
            flags.readable
        };
        if !flags.user || !permitted {
            return Err(UserMemError::PermissionDenied);
        }
        let remaining = PAGE_SIZE - address % PAGE_SIZE;
        if last - address < remaining {
            return Ok(());
        }
        address += remaining;
    }
}

/// Copy a resident user range into a kernel buffer without changing SUM.
///
/// # Safety
/// Mappings and their backing must remain stable until return. Every queried
/// physical byte must have a readable supervisor RAM alias. `dst` must not
/// overlap any of that backing. These conditions require external synchronization.
pub unsafe fn copy_from_user(
    space: &impl AddressSpace,
    dst: &mut [u8],
    user_src: usize,
) -> Result<(), UserMemError> {
    preflight(space, user_src, dst.len(), false)?;
    let mut copied = 0;
    while copied < dst.len() {
        let address = user_src + copied;
        let (physical, _) = space.query(address).expect("stable user mapping");
        let count = (PAGE_SIZE - address % PAGE_SIZE).min(dst.len() - copied);
        unsafe {
            ptr::copy_nonoverlapping(
                phys_map::phys_to_virt(physical).expect("preflight RAM alias") as *const u8,
                dst.as_mut_ptr().add(copied),
                count,
            )
        };
        copied += count;
    }
    Ok(())
}

/// Copy a kernel buffer into a resident user range, preflighting all pages first.
///
/// # Safety
/// Mappings and their backing must remain stable until return. Every queried
/// physical byte must have a writable supervisor RAM alias. `src` must not
/// overlap any of that backing. These conditions require external synchronization.
pub unsafe fn copy_to_user(
    space: &impl AddressSpace,
    user_dst: usize,
    src: &[u8],
) -> Result<(), UserMemError> {
    preflight(space, user_dst, src.len(), true)?;
    let mut copied = 0;
    while copied < src.len() {
        let address = user_dst + copied;
        let (physical, _) = space.query(address).expect("stable user mapping");
        let count = (PAGE_SIZE - address % PAGE_SIZE).min(src.len() - copied);
        unsafe {
            ptr::copy_nonoverlapping(
                src.as_ptr().add(copied),
                phys_map::phys_to_virt(physical).expect("preflight RAM alias") as *mut u8,
                count,
            )
        };
        copied += count;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputError {
    UserMemory,
    Overlap,
}

/// Output spans must be disjoint even when different virtual pages alias RAM.
pub(crate) fn validate_outputs(space: &impl AddressSpace, spans: &[(usize, usize)]) -> Result<(), OutputError> {
    for &(start, len) in spans {
        preflight(space, start, len, true).map_err(|_| OutputError::UserMemory)?;
    }
    for (index, &(start, len)) in spans.iter().enumerate() {
        if len == 0 {
            continue;
        }
        let end = start + len;
        for &(other, count) in &spans[..index] {
            if count != 0 && start < other + count && other < end {
                return Err(OutputError::Overlap);
            }
        }
        // Compare each physical chunk to every preceding chunk, including its own span.
        let mut offset = 0;
        while offset < len {
            let address = start + offset;
            let physical = space.query(address).expect("preflight mapping").0;
            let count = (4096 - address % 4096).min(len - offset);
            for &(other, length) in &spans[..=index] {
                let limit = if other == start && length == len {
                    offset
                } else {
                    length
                };
                let mut preceding = 0;
                while preceding < limit {
                    let other_address = other + preceding;
                    let other_physical = space.query(other_address).expect("preflight mapping").0;
                    let other_count = (4096 - other_address % 4096).min(limit - preceding);
                    if physical < other_physical + other_count && other_physical < physical + count
                    {
                        return Err(OutputError::Overlap);
                    }
                    preceding += other_count;
                }
            }
            offset += count;
        }
    }
    Ok(())
}

/// Boot checks use detached page tables; no test user address is dereferenced.
pub fn self_test() {
    use super::bitmap;
    use super::paging::{ArchAddressSpace, MapFlags};
    use super::vmo::Vmo;

    let before = bitmap::stats().free_pages;
    for create in [Vmo::create, Vmo::create_contiguous] {
        let dirty = create(PAGE_SIZE + 1).expect("dirty VMO allocation");
        let first = dirty.frames().get(0).unwrap();
        for frame in dirty.frames().iter() {
            unsafe {
                ptr::write_bytes(
                    phys_map::phys_to_virt(frame).unwrap() as *mut u8,
                    0xa5,
                    PAGE_SIZE,
                )
            };
        }
        let physical = Vmo::create_physical(first, PAGE_SIZE).unwrap();
        let mut byte = [0];
        physical.read(0, &mut byte).unwrap();
        assert_eq!(byte, [0xa5], "physical VMO must not clear memory");
        drop(physical);
        drop(dirty);
        let clean = create(PAGE_SIZE + 1).expect("recycled VMO allocation");
        assert_eq!(clean.frames().get(0), Some(first));
        for frame in clean.frames().iter() {
            let page = unsafe {
                core::slice::from_raw_parts(
                    phys_map::phys_to_virt(frame).unwrap() as *const u8,
                    PAGE_SIZE,
                )
            };
            assert!(
                page.iter().all(|&byte| byte == 0),
                "VMO page tail leaked old data"
            );
        }
    }

    const BASE: usize = 0x30_0000_0000;
    let mut backing = Vmo::create(PAGE_SIZE * 2).expect("copy test backing");
    let mut space = ArchAddressSpace::new().expect("copy test address space");
    // SAFETY: detached mappings and private RAM backing remain live throughout all copies.
    unsafe {
        space
            .map_vmo(BASE, backing.frames(), MapFlags::user_rw())
            .unwrap();
        let address = BASE + PAGE_SIZE - 3;
        let data = [0x13, 0x27, 0x39, 0x4b, 0x5d, 0x6f, 0x71];
        copy_to_user(&space, address, &data).unwrap();
        let mut output = [0; 7];
        copy_from_user(&space, &mut output, address).unwrap();
        assert_eq!(output, data);
        backing.read(PAGE_SIZE - 3, &mut output).unwrap();
        assert_eq!(output, data, "translation offset was applied twice");

        assert_eq!(copy_from_user(&space, &mut [], 0), Ok(()));
        assert_eq!(copy_to_user(&space, usize::MAX, &[]), Ok(()));
        for invalid in [0, USER_MAX + 1, usize::MAX] {
            let mut output = [0x88; 2];
            assert_eq!(
                copy_from_user(&space, &mut output, invalid),
                Err(UserMemError::InvalidRange)
            );
            assert_eq!(output, [0x88; 2]);
            assert_eq!(
                copy_to_user(&space, invalid, &[1, 2]),
                Err(UserMemError::InvalidRange)
            );
        }
        assert_eq!(
            copy_to_user(&space, USER_MAX, &[1, 2]),
            Err(UserMemError::InvalidRange)
        );
        assert_eq!(
            copy_to_user(&space, BASE + PAGE_SIZE * 2, &[1]),
            Err(UserMemError::NotMapped)
        );
        let mut unmapped = [0x88];
        assert_eq!(
            copy_from_user(&space, &mut unmapped, BASE + PAGE_SIZE * 2),
            Err(UserMemError::NotMapped)
        );
        assert_eq!(unmapped, [0x88]);

        space
            .protect_page(BASE + PAGE_SIZE, MapFlags::user_ro())
            .unwrap();
        assert_eq!(
            copy_to_user(&space, address, &[0xff; 7]),
            Err(UserMemError::PermissionDenied)
        );
        backing.read(PAGE_SIZE - 3, &mut output).unwrap();
        assert_eq!(
            output, data,
            "read-only second page allowed a partial write"
        );
        copy_from_user(&space, &mut output, address).unwrap();
        assert_eq!(output, data);

        space
            .protect_page(BASE + PAGE_SIZE, MapFlags::kernel_rw())
            .unwrap();
        output.fill(0x88);
        assert_eq!(
            copy_from_user(&space, &mut output, address),
            Err(UserMemError::PermissionDenied)
        );
        assert_eq!(
            output, [0x88; 7],
            "supervisor second page allowed a partial read"
        );
        assert_eq!(
            copy_to_user(&space, address, &[0xff; 7]),
            Err(UserMemError::PermissionDenied)
        );

        #[cfg(target_arch = "riscv64")]
        {
            let execute_only = MapFlags {
                readable: false,
                writable: false,
                executable: true,
                user: true,
            };
            space.protect_page(BASE + PAGE_SIZE, execute_only).unwrap();
            assert_eq!(
                copy_from_user(&space, &mut output, address),
                Err(UserMemError::PermissionDenied)
            );
            assert_eq!(output, [0x88; 7]);
        }

        space.unmap_page(BASE + PAGE_SIZE).unwrap();
        assert_eq!(
            copy_from_user(&space, &mut output, address),
            Err(UserMemError::NotMapped)
        );
        assert_eq!(
            output, [0x88; 7],
            "unmapped second page allowed a partial read"
        );
        assert_eq!(
            copy_to_user(&space, address, &[0xff; 7]),
            Err(UserMemError::NotMapped)
        );
        backing.read(PAGE_SIZE - 3, &mut output).unwrap();
        assert_eq!(output, data, "unmapped second page allowed a partial write");

        space
            .map_page(
                USER_MAX & !(PAGE_SIZE - 1),
                backing.frames().get(1).unwrap(),
                MapFlags::user_rw(),
            )
            .unwrap();
        copy_to_user(&space, USER_MAX, &[0x42]).unwrap();
        let mut last = [0];
        copy_from_user(&space, &mut last, USER_MAX).unwrap();
        assert_eq!(last, [0x42], "last canonical user byte must be usable");
    }
    drop(space);
    assert_eq!(
        bitmap::stats().free_pages + backing.page_count(),
        before,
        "address-space drop freed VMO backing or leaked table frames"
    );
    backing.write(0, &[0x19]).unwrap();
    let mut byte = [0];
    backing.read(0, &mut byte).unwrap();
    assert_eq!(byte, [0x19], "page table drop must retain VMO backing");
    drop(backing);
    assert_eq!(
        bitmap::stats().free_pages,
        before,
        "user-copy tests leaked physical frames"
    );
}
