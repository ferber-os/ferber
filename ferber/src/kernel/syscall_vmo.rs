//! Capability-authorized byte operations; staging makes aliased copies transactional.

use super::{allocation_errno, cap_errno, SYS_VMO_CREATE, SYS_VMO_READ, SYS_VMO_WRITE, SYS_VMO_MAP, SYS_VMO_UNMAP, SYS_VMO_SEAL};
use crate::kernel::capabilities::{CapNode, ObjectType, Rights, CAP_NODE_ALLOCATOR};
use crate::kernel::memory::{
    user_mem,
    vmo::{Vmo, VmoError, MAX_USER_VMO_SIZE},
};
use crate::kernel::sync::SpinLock;
use crate::kernel::task::{Process, process::MappingRecord};
use crate::kernel::memory::paging::{AddressSpace, MapFlags, PageError};
use alloc::{sync::Arc, vec::Vec};

#[cfg(not(test))]
#[path = "syscall_vmo_tests.rs"]
mod tests;

pub(super) const VMO_CREATE_MAPPABLE: usize = 1;
pub(super) const VMO_CREATE_EXECUTABLE: usize = 2;
pub(super) const MAX_COPY: usize = 64 * 1024;
pub(super) const INITIAL_RIGHTS: Rights = Rights::READ
    .union(Rights::WRITE)
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER)
    .union(Rights::INSPECT)
    .union(Rights::MANAGE);

fn errno(error: VmoError) -> isize {
    match error {
        VmoError::OutOfMemory => -12isize,
        VmoError::UserBudget | VmoError::JobQuota => -28isize,
        VmoError::TooLarge => -90isize,
        VmoError::PermissionDenied => -13isize,
        VmoError::Busy => -16isize,
        _ => -22isize,
    }
}

pub(super) fn dispatch(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    let result = if nr == SYS_VMO_CREATE {
        create(process, args[0], args[1])
    } else if nr == SYS_VMO_MAP {
        map(process, args).map(|()| 0)
    } else if nr == SYS_VMO_UNMAP {
        unmap(process, args[0], args[1]).map(|()| 0)
    } else if nr == SYS_VMO_SEAL {
        seal(process, args[0], args[1]).map(|()| 0)
    } else {
        copy(process, nr, args).map(|()| 0)
    };
    result.unwrap_or_else(|error| error)
}

fn create(process: &Arc<SpinLock<Process>>, size: usize, options: usize) -> Result<isize, isize> {
    if !matches!(options, 0 | VMO_CREATE_MAPPABLE | 3) || size == 0 {
        return Err(-22isize);
    }
    if size > MAX_USER_VMO_SIZE {
        return Err(-90isize);
    }
    {
        let process = process.lock();
        if process.handle_table().lock().is_full() {
            return Err(-24isize);
        }
    }
    let backing = Vmo::create_user(size).map_err(errno)?;
    let object = crate::kernel::memory::accounting::attempt(|| Arc::try_new(backing)).map_err(allocation_errno)?;
    let mut rights = INITIAL_RIGHTS;
    if options & VMO_CREATE_MAPPABLE != 0 { rights |= Rights::MAP; }
    if options & VMO_CREATE_EXECUTABLE != 0 { rights |= Rights::EXECUTE; }
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(object, ObjectType::Vmo, rights)
        .map_err(cap_errno)?;
    let result = {
        let process = process.lock();
        let mut table = process.handle_table().lock();
        table.insert(node)
    };
    match result {
        Ok(handle) => Ok(handle as isize),
        Err((error, node)) => {
            drop(node);
            Err(cap_errno(error))
        }
    }
}

fn lookup(process: &Process, raw: usize, rights: Rights) -> Result<Arc<CapNode>, isize> {
    let handle = u32::try_from(raw).map_err(|_| -9isize)?;
    let table = process.handle_table().lock();
    let node = table.lookup(handle).map_err(cap_errno)?;
    if !node.rights().contains(rights) {
        return Err(-13isize);
    }
    if node.object_type() != ObjectType::Vmo {
        return Err(-22isize);
    }
    let object = node.object_ref::<Vmo>().ok_or(-22isize)?;
    if !object.is_owned_ram() {
        return Err(-22isize);
    }
    Ok(Arc::clone(node))
}

fn page_errno(error: PageError) -> isize {
    match error {
        PageError::OutOfMemory => -12,
        PageError::TableBudget | PageError::JobQuota => -28,
        PageError::AlreadyMapped => -17,
        _ => -14,
    }
}

fn seal(process: &Arc<SpinLock<Process>>, handle: usize, options: usize) -> Result<(), isize> {
    let node;
    let process = process.lock();
    node = lookup(&process, handle, Rights::MANAGE | Rights::EXECUTE)?;
    if options != 0 { return Err(-22); }
    node.object_ref::<Vmo>().expect("checked VMO backing").seal().map_err(errno)
}

fn map(process: &Arc<SpinLock<Process>>, args: [usize; 6]) -> Result<(), isize> {
    // The object outlives both the process guard and every published PTE.
    let object;
    let mut process = process.lock();
    let node = lookup(&process, args[0], Rights::MAP)?;
    object = node.object_arc::<Vmo>().ok_or(-22isize)?;
    if args[5] != 0 || !matches!(args[4], 1 | 3 | 5) { return Err(-22); }
    let flags = match args[4] { 3 => MapFlags::user_rw(), 5 => MapFlags::user_rx(), _ => MapFlags::user_ro() };
    Vmo::check_mapping_rights(node.rights(), flags).map_err(|_| -13isize)?;
    drop(node);
    let (offset, base, len) = (args[1], args[2], args[3]);
    if len == 0 || (offset | base | len) & 4095 != 0 { return Err(-22); }
    if len > MAX_USER_VMO_SIZE { return Err(-90); }
    object.validate_owned_range(offset, len).map_err(errno)?;
    let limit = if cfg!(target_arch = "riscv64") { 1usize << 38 } else { 1usize << 47 };
    let end = base.checked_add(len).filter(|end| base >= 4096 && *end <= limit).ok_or(-14isize)?;
    if process.mapping_reserved(base, end) { return Err(-22); }
    if process.mappings().iter().any(|r| base < r.base + r.len && r.base < end) {
        return Err(-17);
    }
    process.address_space().check_free_range(base, len / 4096).map_err(page_errno)?;
    let writable = object.mapping_permit(flags).map_err(errno)?;
    if process.mappings().len() >= 64 || process.mappings().iter().map(|r| r.len / 4096).sum::<usize>() + len / 4096 > 16384 {
        return Err(-28);
    }
    crate::kernel::memory::accounting::attempt(|| process.mappings_mut().try_reserve(1)).map_err(allocation_errno)?;
    unsafe { process.address_space_mut().map_pages(base, object.frames(), offset / 4096, len / 4096, flags).map_err(page_errno)?; }
    process.mappings_mut().push(MappingRecord { base, len, offset, flags, object: Arc::clone(&object), _writable: writable });
    Ok(())
}

fn unmap(process: &Arc<SpinLock<Process>>, base: usize, len: usize) -> Result<(), isize> {
    let detached;
    let mut process = process.lock();
    if len == 0 || (base | len) & 4095 != 0 { return Err(-22); }
    let limit = if cfg!(target_arch = "riscv64") { 1usize << 38 } else { 1usize << 47 };
    base.checked_add(len).filter(|end| base >= 4096 && *end <= limit).ok_or(-14isize)?;
    let index = process.mappings().iter().position(|r| r.base == base && r.len == len).ok_or(-22isize)?;
    let record = &process.mappings()[index];
    let (offset, flags, object) = (record.offset, record.flags, Arc::clone(&record.object));
    unsafe { process.address_space_mut().unmap_pages_exact(base, object.frames(), offset / 4096, len / 4096, flags).map_err(|error| if error == PageError::NotMapped { -22 } else { page_errno(error) })?; }
    detached = process.mappings_mut().remove(index);
    drop(process);
    drop(detached);
    Ok(())
}

fn copy(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> Result<(), isize> {
    // Declare retained ownership first so it is released after the mapping guard.
    let node;
    let process = process.lock();
    let reading = nr == SYS_VMO_READ;
    debug_assert!(reading || nr == SYS_VMO_WRITE);
    node = lookup(
        &process,
        args[0],
        if reading { Rights::READ } else { Rights::WRITE },
    )?;
    let object = node.object_ref::<Vmo>().expect("checked VMO backing");
    if args[3] > MAX_COPY {
        return Err(-90isize);
    }
    object
        .validate_owned_range(args[1], args[3])
        .map_err(errno)?;
    if !reading { object.check_writable().map_err(errno)?; }
    if args[3] == 0 {
        return Ok(());
    }
    if reading {
        user_mem::validate_outputs(process.address_space(), &[(args[2], args[3])]).map_err(
            |error| match error {
                user_mem::OutputError::UserMemory => -14isize,
                user_mem::OutputError::Overlap => -22isize,
            },
        )?;
    } else {
        user_mem::preflight(process.address_space(), args[2], args[3], false)
            .map_err(|_| -14isize)?;
    }
    let mut staging = Vec::new();
    crate::kernel::memory::accounting::attempt(|| staging.try_reserve_exact(args[3])).map_err(allocation_errno)?;
    staging.resize(args[3], 0);
    if reading {
        object.read_owned(args[1], &mut staging).map_err(errno)?;
        unsafe {
            user_mem::copy_to_user(process.address_space(), args[2], &staging)
                .expect("preflight pinned VMO output")
        };
    } else {
        unsafe {
            user_mem::copy_from_user(process.address_space(), &mut staging, args[2])
                .expect("preflight pinned VMO input")
        };
        object.write_owned(args[1], &staging).map_err(errno)?;
    }
    Ok(())
}

#[cfg(not(test))]
pub(super) fn self_test() {
    tests::run();
}
