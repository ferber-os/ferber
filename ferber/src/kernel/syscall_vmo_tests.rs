//! Exercise real dispatch against detached mappings and shared capability backing.
use super::*;
use crate::arch::ArchAddressSpace;
use crate::kernel::capabilities::{CapError, Resource};
use crate::kernel::memory::{
    bitmap,
    paging::{AddressSpace, MapFlags},
};
use crate::kernel::syscall::{
    dispatch_for_process, SYS_CAP_ATTENUATE, SYS_HANDLE_CLOSE, SYS_HANDLE_DUPLICATE,
};

const BASE: usize = 0x32_0000_0000;
const ALIAS: usize = BASE + 0x10000;

fn call(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    dispatch_for_process(process, nr, args)
}
fn insert(process: &Arc<SpinLock<Process>>, node: Arc<CapNode>) -> u32 {
    process.lock().handle_table().lock().insert(node).unwrap()
}
fn close(process: &Arc<SpinLock<Process>>, handle: isize) {
    assert_eq!(
        call(process, SYS_HANDLE_CLOSE, [handle as usize, 0, 0, 0, 0, 0]),
        0
    );
}
fn bytes(process: &Arc<SpinLock<Process>>, address: usize) -> [u8; 8] {
    let process = process.lock();
    let mut bytes = [0; 8];
    unsafe {
        user_mem::copy_from_user(process.address_space(), &mut bytes, address).unwrap();
    }
    bytes
}
fn put(process: &Arc<SpinLock<Process>>, address: usize, bytes: &[u8]) {
    let process = process.lock();
    unsafe {
        user_mem::copy_to_user(process.address_space(), address, bytes).unwrap();
    }
}

pub(super) fn run() {
    let frames = bitmap::stats().free_pages;
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    let budget = crate::kernel::memory::vmo::user_budget_stats();
    let backing = Vmo::create(8192).unwrap();
    let mut space = ArchAddressSpace::new().unwrap();
    unsafe {
        space
            .map_vmo(BASE, backing.frames(), MapFlags::user_rw())
            .unwrap();
        space
            .map_page(ALIAS, backing.frames().get(0).unwrap(), MapFlags::user_rw())
            .unwrap();
        space
            .map_page(
                ALIAS + 4096,
                backing.frames().get(0).unwrap(),
                MapFlags::user_rw(),
            )
            .unwrap();
    }
    let process = Arc::new(SpinLock::new(Process::new(
        0x564d,
        "vmo_tests",
        space,
        None,
    )));
    assert_eq!(call(&process, SYS_VMO_CREATE, [0; 6]), -22);
    assert_eq!(call(&process, SYS_VMO_CREATE, [1, 2, 0, 0, 0, 0]), -22);
    assert_eq!(
        call(
            &process,
            SYS_VMO_CREATE,
            [MAX_USER_VMO_SIZE + 1, 0, 0, 0, 0, 0]
        ),
        -90
    );
    let handle = call(&process, SYS_VMO_CREATE, [4097, 0, usize::MAX, 0, 0, 0]);
    assert!(handle >= 0);
    assert_eq!(
        crate::kernel::memory::vmo::user_budget_stats().1,
        budget.1 + 2
    );
    {
        let process = process.lock();
        let table = process.handle_table().lock();
        let node = table.lookup(handle as u32).unwrap();
        assert_eq!(node.rights(), INITIAL_RIGHTS);
        assert_eq!(node.object_ref::<Vmo>().unwrap().size(), 4097);
    }
    put(&process, BASE, &[0xa5; 8]);
    assert_eq!(
        call(&process, SYS_VMO_READ, [handle as usize, 0, BASE, 8, 0, 0]),
        0
    );
    assert_eq!(bytes(&process, BASE), [0; 8]);
    put(&process, BASE, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        call(
            &process,
            SYS_VMO_WRITE,
            [handle as usize, 4089, BASE, 8, 0, 0]
        ),
        0
    );
    let duplicate = call(
        &process,
        SYS_HANDLE_DUPLICATE,
        [handle as usize, 0, 0, 0, 0, 0],
    );
    assert!(duplicate >= 0);
    assert_eq!(
        call(
            &process,
            SYS_VMO_READ,
            [duplicate as usize, 4089, BASE + 64, 8, 0, 0]
        ),
        0
    );
    assert_eq!(bytes(&process, BASE + 64), [1, 2, 3, 4, 5, 6, 7, 8]);
    let child = call(
        &process,
        SYS_CAP_ATTENUATE,
        [handle as usize, Rights::READ.bits() as usize, 0, 0, 0, 0],
    );
    assert!(child >= 0);
    assert_eq!(
        call(&process, SYS_VMO_WRITE, [child as usize, 0, BASE, 0, 0, 0]),
        -13
    );
    for (offset, len, expected) in [
        (4097, 0, 0),
        (4098, 0, -22),
        (4097, 1, -22),
        (usize::MAX, 1, -22),
        (0, MAX_COPY + 1, -90),
    ] {
        assert_eq!(
            call(
                &process,
                SYS_VMO_READ,
                [handle as usize, offset, usize::MAX, len, 0, 0]
            ),
            expected
        );
    }
    assert_eq!(
        call(&process, SYS_VMO_READ, [1usize << 32, 0, BASE, 0, 0, 0]),
        -9
    );
    assert_eq!(call(&process, SYS_VMO_READ, [9999, 0, BASE, 0, 0, 0]), -9);
    // A later invalid page must not change any preceding destination bytes.
    put(&process, BASE + 4092, &[0x88; 8]);
    unsafe {
        process
            .lock()
            .address_space_mut()
            .protect_page(BASE + 4096, MapFlags::user_ro())
            .unwrap();
    }
    assert_eq!(
        call(
            &process,
            SYS_VMO_READ,
            [handle as usize, 0, BASE + 4092, 8, 0, 0]
        ),
        -14
    );
    assert_eq!(bytes(&process, BASE + 4092), [0x88; 8]);
    unsafe {
        process
            .lock()
            .address_space_mut()
            .unmap_page(BASE + 4096)
            .unwrap();
    }
    assert_eq!(
        call(
            &process,
            SYS_VMO_WRITE,
            [handle as usize, 0, BASE + 4092, 8, 0, 0]
        ),
        -14
    );
    assert_eq!(
        call(&process, SYS_VMO_READ, [handle as usize, 0, BASE, 8, 0, 0]),
        0
    );
    assert_eq!(bytes(&process, BASE), [0; 8]);
    assert_eq!(
        call(
            &process,
            SYS_VMO_READ,
            [handle as usize, 0, ALIAS + 4092, 4097, 0, 0]
        ),
        -22
    );
    // Physical wrappers are forbidden even when their pages are ordinary RAM.
    let physical = Arc::new(Vmo::create_physical(backing.frames().get(0).unwrap(), 4096).unwrap());
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(physical, ObjectType::Vmo, INITIAL_RIGHTS)
        .unwrap();
    let raw = insert(&process, node);
    assert_eq!(
        call(&process, SYS_VMO_READ, [raw as usize, 0, 0, 0, 0, 0]),
        -22
    );
    close(&process, raw as isize);
    for (object, kind) in [
        (
            Arc::new(Resource) as Arc<dyn crate::kernel::capabilities::KernelObject>,
            ObjectType::Vmo,
        ),
        (Arc::new(Vmo::create(1).unwrap()), ObjectType::Resource),
    ] {
        let node = CAP_NODE_ALLOCATOR
            .alloc_root(object, kind, INITIAL_RIGHTS)
            .unwrap();
        let raw = insert(&process, node);
        assert_eq!(
            call(&process, SYS_VMO_READ, [raw as usize, 0, 0, 0, 0, 0]),
            -22
        );
        close(&process, raw as isize);
    }
    // Different virtual addresses may name this VMO itself: snapshot before writing.
    let shared = Arc::new(Vmo::create_user(8192).unwrap());
    shared.write_owned(0, &[1, 2, 3, 4, 5, 6, 7, 8, 9]).unwrap();
    unsafe {
        process
            .lock()
            .address_space_mut()
            .map_vmo(BASE + 0x20000, shared.frames(), MapFlags::user_rw())
            .unwrap();
    }
    let shared_node = CAP_NODE_ALLOCATOR
        .alloc_root(shared.clone(), ObjectType::Vmo, INITIAL_RIGHTS)
        .unwrap();
    let shared_handle = insert(&process, shared_node);
    assert_eq!(
        call(
            &process,
            SYS_VMO_WRITE,
            [shared_handle as usize, 1, BASE + 0x20000, 8, 0, 0]
        ),
        0
    );
    let mut output = [0; 9];
    shared.read_owned(0, &mut output).unwrap();
    assert_eq!(output, [1, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        call(
            &process,
            SYS_VMO_READ,
            [shared_handle as usize, 0, BASE + 0x20001, 8, 0, 0]
        ),
        0
    );
    shared.read_owned(0, &mut output).unwrap();
    assert_eq!(output, [1, 1, 1, 2, 3, 4, 5, 6, 7]);
    unsafe {
        process
            .lock()
            .address_space_mut()
            .unmap_vmo(BASE + 0x20000, 2)
            .unwrap();
    }
    close(&process, shared_handle as isize);
    drop(shared);
    let retained = {
        let process = process.lock();
        let table = process.handle_table().lock();
        Arc::clone(table.lookup(handle as u32).unwrap())
    };
    retained.revoke();
    assert_eq!(
        call(&process, SYS_VMO_READ, [child as usize, 0, 0, 0, 0, 0]),
        -116
    );
    assert_eq!(
        process
            .lock()
            .handle_table()
            .lock()
            .lookup(handle as u32)
            .unwrap_err(),
        CapError::Revoked
    );
    close(&process, handle);
    close(&process, duplicate);
    close(&process, child);
    assert_eq!(
        crate::kernel::memory::vmo::user_budget_stats().1,
        budget.1 + 2
    );
    drop(retained);
    assert_eq!(crate::kernel::memory::vmo::user_budget_stats(), budget);
    mapping_tests(&process);
    drop(process);
    drop(backing);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes);
    assert_eq!(bitmap::stats().free_pages, frames);
    crate::kprintln!("[mem]   VMO syscall authority, snapshots, and rollback: ok");
}


fn mapping_tests(process: &Arc<SpinLock<Process>>) {
    const TARGET: usize = BASE + 0x100000;
    let budget = crate::kernel::memory::vmo::user_budget_stats();
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    let handle = call(process, SYS_VMO_CREATE, [8193, VMO_CREATE_MAPPABLE, 0, 0, 0, 0]);
    assert!(handle >= 0);
    let node = {
        let p = process.lock();
        let table = p.handle_table().lock();
        Arc::clone(table.lookup(handle as u32).unwrap())
    };
    assert_eq!(node.rights().bits(), 815);
    let no_map = insert(process, CAP_NODE_ALLOCATOR.alloc_root(
        Arc::new(Vmo::create(4096).unwrap()), ObjectType::Vmo, INITIAL_RIGHTS).unwrap());
    assert_eq!(call(process, SYS_VMO_MAP, [no_map as usize, 0, TARGET, 4096, 1, 0]), -13);
    close(process, no_map as isize);
    let read_only = insert(process, node.attenuate(INITIAL_RIGHTS.union(Rights::MAP).difference(Rights::WRITE)).unwrap());
    assert_eq!(call(process, SYS_VMO_MAP, [read_only as usize, 0, TARGET, 4096, 3, 0]), -13);
    close(process, read_only as isize);
    let wrong_type = insert(process, CAP_NODE_ALLOCATOR.alloc_root(
        Arc::new(Resource), ObjectType::Vmo, INITIAL_RIGHTS.union(Rights::MAP)).unwrap());
    assert_eq!(call(process, SYS_VMO_MAP, [wrong_type as usize, 0, TARGET, 4096, 1, 0]), -22);
    close(process, wrong_type as isize);
    let physical = insert(process, CAP_NODE_ALLOCATOR.alloc_root(
        Arc::new(Vmo::create_physical(0, 4096).unwrap()), ObjectType::Vmo, INITIAL_RIGHTS.union(Rights::MAP)).unwrap());
    assert_eq!(call(process, SYS_VMO_MAP, [physical as usize, 0, TARGET, 4096, 1, 0]), -22);
    close(process, physical as isize);
    // Reservations exclude both an unmapped stack and its preceding guard.
    let thread = process.lock().create_thread(0, 4096).unwrap();
    thread.lock().set_user_stack(Vmo::create(4096).unwrap());
    thread.lock().set_user_stack_mapping(TARGET + 0x80000);
    for address in [TARGET + 0x7f000, TARGET + 0x80000] {
        assert_eq!(call(process, SYS_VMO_MAP, [handle as usize, 0, address, 4096, 1, 0]), -22);
    }
    let id = thread.lock().id();
    let detached = process.lock().detach_thread(id);
    drop(detached);
    drop(thread);
    let map = |offset, address, len, permissions| call(process, SYS_VMO_MAP, [handle as usize, offset, address, len, permissions, 0]);
    assert_eq!(map(0, TARGET, 4096, 5), -22);
    assert_eq!(map(8192, TARGET, 4096, 1), -22);
    assert_eq!(map(0, 0, 4096, 1), -14);
    assert_eq!(map(0, TARGET, 4096, 3), 0);
    assert_eq!(map(0, TARGET, 4096, 1), -17);
    assert_eq!(map(4096, TARGET + 8192, 4096, 1), 0);
    let (physical, flags) = process.lock().address_space().query(TARGET + 8192).unwrap();
    assert_eq!(physical, node.object_ref::<Vmo>().unwrap().frames().get(1).unwrap());
    assert_eq!(flags, MapFlags::user_ro());
    assert_eq!(call(process, SYS_VMO_UNMAP, [TARGET, 8192, 0, 0, 0, 0]), -22);
    put(process, TARGET, &[0x5a; 8]);
    node.revoke();
    assert_eq!(map(0, TARGET + 16384, 4096, 1), -116);
    close(process, handle);
    drop(node);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes);
    assert_eq!(bytes(process, TARGET), [0x5a; 8]);
    assert_eq!(crate::kernel::memory::vmo::user_budget_stats().1, budget.1 + 3);
    assert_eq!(call(process, SYS_VMO_UNMAP, [TARGET, 4096, 0, 0, 0, 0]), 0);
    assert_eq!(call(process, SYS_VMO_UNMAP, [TARGET + 8192, 4096, 0, 0, 0, 0]), 0);
    assert_eq!(crate::kernel::memory::vmo::user_budget_stats(), budget);

    let handle = call(process, SYS_VMO_CREATE, [4096, VMO_CREATE_MAPPABLE, 0, 0, 0, 0]);
    for i in 0..64 {
        assert_eq!(call(process, SYS_VMO_MAP, [handle as usize, 0, TARGET + i * 4096, 4096, 1, 0]), 0);
    }
    assert_eq!(call(process, SYS_VMO_MAP, [handle as usize, 0, TARGET + 64 * 4096, 4096, 1, 0]), -28);
    close(process, handle);
    let detached = process.lock().take_mappings();
    assert!(process.lock().address_space().query(TARGET).is_none());
    assert_eq!(crate::kernel::memory::vmo::user_budget_stats().1, budget.1 + 1);
    drop(detached);
    assert_eq!(crate::kernel::memory::vmo::user_budget_stats(), budget);
}
