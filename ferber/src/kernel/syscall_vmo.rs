//! Capability-authorized byte operations; staging makes aliased copies transactional.

use super::{cap_errno, SYS_VMO_CREATE, SYS_VMO_READ, SYS_VMO_WRITE};
use crate::kernel::capabilities::{CapNode, ObjectType, Rights, CAP_NODE_ALLOCATOR};
use crate::kernel::memory::{
    user_mem,
    vmo::{Vmo, VmoError, MAX_USER_VMO_SIZE},
};
use crate::kernel::sync::SpinLock;
use crate::kernel::task::Process;
use alloc::{sync::Arc, vec::Vec};

#[cfg(not(test))]
#[path = "syscall_vmo_tests.rs"]
mod tests;

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
        VmoError::UserBudget => -28isize,
        VmoError::TooLarge => -90isize,
        _ => -22isize,
    }
}

pub(super) fn dispatch(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    let result = if nr == SYS_VMO_CREATE {
        create(process, args[0], args[1])
    } else {
        copy(process, nr, args).map(|()| 0)
    };
    result.unwrap_or_else(|error| error)
}

fn create(process: &Arc<SpinLock<Process>>, size: usize, options: usize) -> Result<isize, isize> {
    if options != 0 || size == 0 {
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
    let object = Arc::try_new(Vmo::create_user(size).map_err(errno)?).map_err(|_| -12isize)?;
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(object, ObjectType::Vmo, INITIAL_RIGHTS)
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
    staging.try_reserve_exact(args[3]).map_err(|_| -12isize)?;
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
