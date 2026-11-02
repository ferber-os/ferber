//! Device handles select approved apertures; userspace never supplies physical addresses.
use crate::kernel::syscall::{allocation_errno, cap_errno};
use crate::kernel::{
    capabilities::{CapNode, ObjectType, Rights},
    device::{abi, Assignment},
    memory::{accounting, paging::{AddressSpace, MapFlags, PageError}, user_mem, vmo::Vmo},
    sync::SpinLock,
    task::{Process, process::{DeviceMappingRecord, OwnedDevice}},
};
use alloc::sync::Arc;

fn page_errno(error: PageError) -> isize {
    match error {
        PageError::OutOfMemory => -12,
        PageError::TableBudget | PageError::JobQuota => -28,
        PageError::AlreadyMapped => -17,
        _ => -14,
    }
}
fn lookup(process: &Process, handle: usize, rights: Rights) -> Result<Arc<CapNode>, isize> {
    let handle = u32::try_from(handle).map_err(|_| -9isize)?;
    let table = process.handle_table().lock();
    let node = table.lookup(handle).map_err(cap_errno)?;
    if !node.rights().contains(rights) { return Err(-13); }
    if node.object_type() != ObjectType::Device || node.object_ref::<Assignment>().is_none() { return Err(-22); }
    Ok(Arc::clone(node))
}
fn object(node: &Arc<CapNode>) -> Arc<Assignment> {
    node.object_arc::<Assignment>().expect("checked device backing")
}
fn remember(process: &Arc<SpinLock<Process>>, guard: &mut Process, node: &Arc<CapNode>, assignment: &Arc<Assignment>) -> Result<(), isize> {
    assignment.configurable(guard.id()).map_err(|e| e.errno())?;
    if !guard.owned_devices().iter().any(|r| Arc::ptr_eq(&r.node, node)) {
        if guard.owned_devices().len() == 64 { return Err(-28); }
        accounting::attempt(|| guard.owned_devices_mut().try_reserve(1)).map_err(allocation_errno)?;
        crate::kernel::device::runtime::register(process).map_err(|e| e.errno())?;
        assignment.claim(guard.id()).map_err(|e| e.errno())?;
        guard.owned_devices_mut().push(OwnedDevice { assignment: Arc::clone(assignment), node: Arc::clone(node) });
    }
    Ok(())
}
pub(crate) fn dispatch(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    let result = match nr {
        abi::SYS_DEVICE_QUERY => query(process, args),
        abi::SYS_DEVICE_MAP => map(process, args),
        abi::SYS_DEVICE_UNMAP => unmap(process, args),
        abi::SYS_DMA_MAP => dma_map(process, args),
        abi::SYS_DMA_UNMAP => dma_unmap(process, args),
        abi::SYS_DEVICE_CONTROL => control(process, args),
        // Interrupt routing is unavailable until the platform establishes an armed source.
        abi::SYS_IRQ_BIND | abi::SYS_IRQ_ACK => Err(-95),
        _ => Err(-38),
    };
    result.map(|()| 0).unwrap_or_else(|e| e)
}
fn query(process: &Arc<SpinLock<Process>>, a: [usize; 6]) -> Result<(), isize> {
    if a[3] != abi::REGION_INFO_SIZE || a[4] | a[5] != 0 { return Err(-22); }
    let guard = process.lock();
    let node = lookup(&guard, a[0], Rights::INSPECT)?;
    let bytes = object(&node).region(a[1]).map_err(|e| e.errno())?.info().encode().map_err(|_| -22isize)?;
    unsafe { user_mem::copy_to_user(guard.address_space(), a[2], &bytes).map_err(|_| -14isize) }
}
fn map(process: &Arc<SpinLock<Process>>, a: [usize; 6]) -> Result<(), isize> {
    if a[2] != abi::MAP_REQUEST_SIZE || a[3] | a[4] | a[5] != 0 { return Err(-22); }
    let mut guard = process.lock();
    let node = lookup(&guard, a[0], Rights::MAP)?;
    let assignment = object(&node);
    let mut bytes = [0; abi::MAP_REQUEST_SIZE];
    unsafe { user_mem::copy_from_user(guard.address_space(), &mut bytes, a[1]).map_err(|_| -14isize)?; }
    let request = abi::MapRequest::decode(&bytes).map_err(|_| -22isize)?;
    let rights = if request.permissions == 3 { Rights::READ | Rights::WRITE } else { Rights::READ };
    if !node.rights().contains(rights) { return Err(-13); }
    assignment.region(request.region as usize).map_err(|e| e.errno())?.authorize(request).map_err(|e| e.errno())?;
    let base = request.address;
    let len = request.length;
    let limit = if cfg!(target_arch = "riscv64") { 1usize << 38 } else { 1usize << 47 };
    let end = base.checked_add(len).filter(|end| base >= 4096 && *end <= limit).ok_or(-14isize)?;
    if guard.mapping_reserved(base, end) { return Err(-22); }
    guard.address_space().check_free_range(base, len / 4096).map_err(page_errno)?;
    // Device apertures have a separate budget from owned RAM VMO mappings.
    if guard.device_mappings().len() >= 16 || guard.device_mappings().iter().map(|r| r.len / 4096).sum::<usize>()
        .checked_add(len / 4096).is_none_or(|n| n > 32768) { return Err(-28); }
    accounting::attempt(|| guard.device_mappings_mut().try_reserve(1)).map_err(allocation_errno)?;
    remember(process, &mut guard, &node, &assignment)?;
    let physical = assignment.authorize_mapping(guard.id(), request.region as usize, request).map_err(|e| e.errno())?;
    let backing = Vmo::create_physical(physical, len).map_err(|_| -22isize)?;
    let flags = if request.permissions == 3 { MapFlags::user_rw() } else { MapFlags::user_ro() };
    unsafe { guard.address_space_mut().map_device_pages(base, physical, len / 4096, flags).map_err(page_errno)?; }
    guard.device_mappings_mut().push(DeviceMappingRecord { base, len, backing, flags, assignment, node });
    Ok(())
}
fn unmap(process: &Arc<SpinLock<Process>>, a: [usize; 6]) -> Result<(), isize> {
    if a[3] | a[4] | a[5] != 0 { return Err(-22); }
    let mut guard = process.lock();
    let node = lookup(&guard, a[0], Rights::MAP)?;
    let assignment = object(&node);
    assignment.configurable(guard.id()).map_err(|e| e.errno())?;
    let index = guard.device_mappings().iter().position(|r| r.base == a[1] && r.len == a[2]
        && Arc::ptr_eq(&r.assignment, &assignment)).ok_or(-22isize)?;
    let record = &guard.device_mappings()[index];
    let (base, len, physical, flags) = (record.base, record.len, record.backing.frames().get(0).expect("device frame"), record.flags);
    let frames = crate::kernel::memory::vmo::VmoFrames::contiguous(physical, len / 4096);
    unsafe { guard.address_space_mut().unmap_pages_exact(base, &frames, 0, len / 4096, flags).map_err(page_errno)?; }
    guard.device_mappings_mut().remove(index);
    Ok(())
}
fn dma_map(process: &Arc<SpinLock<Process>>, a: [usize; 6]) -> Result<(), isize> {
    if a[3] != abi::DMA_REQUEST_SIZE || a[5] != 24 { return Err(-22); }
    let mut guard = process.lock();
    let node = lookup(&guard, a[0], Rights::MANAGE)?;
    let assignment = object(&node);
    let (backing, rights) = {
        let handle = u32::try_from(a[1]).map_err(|_| -9isize)?;
        let table = guard.handle_table().lock();
        let source = table.lookup(handle).map_err(cap_errno)?;
        if source.object_type() != ObjectType::Vmo { return Err(-22); }
        (source.object_arc::<Vmo>().ok_or(-22isize)?, source.rights())
    };
    let mut bytes = [0; abi::DMA_REQUEST_SIZE];
    unsafe { user_mem::copy_from_user(guard.address_space(), &mut bytes, a[2]).map_err(|_| -14isize)?; }
    let request = abi::DmaRequest::decode(&bytes).map_err(|_| -22isize)?;
    let mut required = Rights::MAP;
    if request.permissions & abi::DEVICE_READ != 0 { required |= Rights::READ; }
    if request.permissions & abi::DEVICE_WRITE != 0 { required |= Rights::WRITE; }
    if !rights.contains(required) { return Err(-13); }
    backing.validate_owned_range(request.offset, request.length).map_err(|_| -22isize)?;
    if request.permissions & abi::DEVICE_WRITE != 0 { backing.check_writable().map_err(|_| -13isize)?; }
    user_mem::preflight(guard.address_space(), a[4], 24, true).map_err(|_| -14isize)?;
    remember(process, &mut guard, &node, &assignment)?;
    let mapping = assignment.map_dma(guard.id(), backing, rights, request).map_err(|e| e.errno())?;
    let mut out = [0; 24];
    out[..8].copy_from_slice(&mapping.id.to_le_bytes());
    out[8..16].copy_from_slice(&(mapping.iova as u64).to_le_bytes());
    out[16..].copy_from_slice(&(mapping.length as u64).to_le_bytes());
    unsafe { user_mem::copy_to_user(guard.address_space(), a[4], &out).expect("preflight pinned DMA output"); }
    Ok(())
}
fn dma_unmap(process: &Arc<SpinLock<Process>>, a: [usize; 6]) -> Result<(), isize> {
    if a[2] | a[3] | a[4] | a[5] != 0 { return Err(-22); }
    let guard = process.lock();
    let node = lookup(&guard, a[0], Rights::MANAGE)?;
    object(&node).unmap_dma(guard.id(), a[1] as u64).map_err(|e| e.errno())
}
fn control(process: &Arc<SpinLock<Process>>, a: [usize; 6]) -> Result<(), isize> {
    if a[3] | a[4] | a[5] != 0 { return Err(-22); }
    let mut guard = process.lock();
    let node = lookup(&guard, a[0], Rights::MANAGE)?;
    let assignment = object(&node);
    match a[1] {
        abi::CONTROL_START if a[2] == 0 => {
            remember(process, &mut guard, &node, &assignment)?;
            assignment.start(guard.id()).map_err(|e| e.errno())
        },
        abi::CONTROL_STOP if a[2] == 0 => assignment.stop(guard.id()).map_err(|e| e.errno()),
        abi::CONTROL_TIME | abi::CONTROL_DEADLINE => Err(-95),
        _ => Err(-22),
    }
}
