//! Dispatcher checks use detached address spaces and never dereference user VAs.

use super::*;
use crate::arch::ArchAddressSpace;
use crate::kernel::capabilities::{CapError, Resource};
use crate::kernel::memory::{bitmap, paging::MapFlags, Vmo};
use crate::kernel::syscall::{dispatch_for_process, SYS_HANDLE_CLOSE};
use crate::kprintln;

const BASE: usize = 0x31_0000_0000;
const ALIAS: usize = BASE + 0x10000;
const DATA: usize = BASE + 64;
const IDS: usize = BASE + 512;
const COUNTS: usize = BASE + 1024;

fn call(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    dispatch_for_process(process, nr, args)
}

fn write(process: &Arc<SpinLock<Process>>, address: usize, bytes: &[u8]) {
    let process = process.lock();
    unsafe { copy_to_user(process.address_space(), address, bytes).unwrap() };
}

fn read<const N: usize>(process: &Arc<SpinLock<Process>>, address: usize) -> [u8; N] {
    let process = process.lock();
    let mut bytes = [0; N];
    unsafe { copy_from_user(process.address_space(), &mut bytes, address).unwrap() };
    bytes
}

fn receive(process: &Arc<SpinLock<Process>>, endpoint: u32) -> isize {
    call(
        process,
        SYS_CHAN_RECV,
        [endpoint as usize, DATA, 64, IDS, 64, COUNTS],
    )
}

fn send(process: &Arc<SpinLock<Process>>, endpoint: u32, length: usize, handles: usize) -> isize {
    call(
        process,
        SYS_CHAN_SEND,
        [endpoint as usize, DATA, length, IDS, handles, 0],
    )
}

fn close(process: &Arc<SpinLock<Process>>, handle: u32) {
    assert_eq!(
        call(process, SYS_HANDLE_CLOSE, [handle as usize, 0, 0, 0, 0, 0]),
        0
    );
}

fn insert(process: &Arc<SpinLock<Process>>, node: Arc<CapNode>) -> u32 {
    process.lock().handle_table().lock().insert(node).unwrap()
}

fn resource(rights: Rights) -> Arc<CapNode> {
    CAP_NODE_ALLOCATOR
        .alloc_root(Arc::new(Resource), ObjectType::Resource, rights)
        .unwrap()
}

pub(super) fn run() {
    let nodes_before = CAP_NODE_ALLOCATOR.live_count();
    let frames_before = bitmap::stats().free_pages;
    let backing = Vmo::create(8192).unwrap();
    let mut space = ArchAddressSpace::new().unwrap();
    unsafe {
        space
            .map_vmo(BASE, backing.frames(), MapFlags::user_rw())
            .unwrap();
        space
            .map_page(ALIAS, backing.frames().get(0).unwrap(), MapFlags::user_rw())
            .unwrap();
    }
    let process = Arc::new(SpinLock::new(Process::new(
        0x4348,
        "channel_tests",
        space,
        None,
    )));
    assert_eq!(process.lock().handle_table().lock().count(), 0);
    assert_eq!(call(&process, SYS_CHAN_CREATE, [0; 6]), -14);
    assert_eq!(process.lock().handle_table().lock().count(), 0);
    assert_eq!(call(&process, SYS_CHAN_CREATE, [BASE, 0, 0, 0, 0, 0]), 0);
    let pair = read::<8>(&process, BASE);
    let first = u32::from_le_bytes(pair[..4].try_into().unwrap());
    let second = u32::from_le_bytes(pair[4..].try_into().unwrap());
    assert_eq!((first, second), (0, 1));
    assert_eq!(
        process
            .lock()
            .handle_table()
            .lock()
            .inspect(first)
            .unwrap()
            .rights,
        ENDPOINT_RIGHTS.bits()
    );
    assert_eq!(receive(&process, second), -11);
    blocking_receive_checks(&process, first, second);
    wait_many_checks(&process, first, second);
    assert_eq!(
        call(&process, SYS_CHAN_SEND, [usize::MAX, 0, 0, 0, 0, 0]),
        -9
    );
    assert_eq!(send(&process, first, DEFAULT_MAX_MESSAGE_BYTES + 1, 0), -90);
    assert_eq!(send(&process, first, 0, DEFAULT_MAX_HANDLES + 1), -90);
    assert_eq!(
        call(&process, SYS_CHAN_SEND, [first as usize, 0, 1, 0, 0, 0]),
        -14
    );
    assert_eq!(
        call(&process, SYS_CHAN_SEND, [first as usize, 0, 0, 0, 0, 0]),
        0
    );
    assert_eq!(
        call(
            &process,
            SYS_CHAN_RECV,
            [second as usize, 0, 0, 0, 0, COUNTS]
        ),
        0
    );
    assert_eq!(read::<8>(&process, COUNTS), [0; 8]);

    write(&process, DATA, b"tungsten");
    assert_eq!(send(&process, first, 8, 0), 0);
    write(&process, DATA, &[0xa5; 8]);
    write(&process, COUNTS, &[0xb6; 8]);
    for (args, error) in [
        ([second as usize, DATA, 7, IDS, 64, COUNTS], -90),
        ([second as usize, DATA, 8, IDS, 64, 0], -14),
        ([second as usize, DATA, 8, IDS, 64, DATA], -22),
        ([second as usize, DATA, 8, IDS, 64, ALIAS + 64], -22),
        ([second as usize, BASE + 8190, 8, IDS, 64, COUNTS], -14),
    ] {
        assert_eq!(call(&process, SYS_CHAN_RECV, args), error);
        assert_eq!(read::<8>(&process, DATA), [0xa5; 8]);
        assert_eq!(read::<8>(&process, COUNTS), [0xb6; 8]);
    }
    assert_eq!(receive(&process, second), 0);
    assert_eq!(read::<8>(&process, DATA), *b"tungsten");
    assert_eq!(read::<8>(&process, COUNTS), [8, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(send(&process, second, 8, 0), 0);
    assert_eq!(receive(&process, first), 0);

    let denied = insert(&process, resource(Rights::READ));
    write(&process, IDS, &denied.to_le_bytes());
    assert_eq!(send(&process, first, 0, 1), -13);
    assert!(process.lock().handle_table().lock().lookup(denied).is_ok());
    assert_eq!(receive(&process, denied), -22);
    close(&process, denied);

    let source = resource(Rights::TRANSFER | Rights::READ | Rights::DUPLICATE);
    let handle = insert(&process, Arc::clone(&source));
    let alias = process
        .lock()
        .handle_table()
        .lock()
        .duplicate(handle)
        .unwrap();
    write(
        &process,
        IDS,
        &[handle.to_le_bytes(), handle.to_le_bytes()].concat(),
    );
    assert_eq!(send(&process, first, 0, 2), -22);
    assert!(process.lock().handle_table().lock().lookup(handle).is_ok());
    write(
        &process,
        IDS,
        &[handle.to_le_bytes(), alias.to_le_bytes()].concat(),
    );
    assert_eq!(send(&process, first, 0, 2), 0);
    assert!(matches!(
        process.lock().handle_table().lock().lookup(handle),
        Err(CapError::InvalidHandle)
    ));
    assert_eq!(
        call(
            &process,
            SYS_CHAN_RECV,
            [second as usize, DATA, 64, IDS, 1, COUNTS]
        ),
        -90
    );
    assert_eq!(receive(&process, second), 0);
    let ids = read::<8>(&process, IDS);
    for chunk in ids.as_chunks::<4>().0 {
        let received = u32::from_le_bytes(*chunk);
        assert!(Arc::ptr_eq(
            process
                .lock()
                .handle_table()
                .lock()
                .lookup(received)
                .unwrap(),
            &source
        ));
        close(&process, received);
    }
    assert!(source.is_valid());

    // The tag and actual backing are both checked; neither can smuggle an endpoint.
    write(&process, IDS, &first.to_le_bytes());
    assert_eq!(send(&process, first, 0, 1), -22);
    let fake = CAP_NODE_ALLOCATOR
        .alloc_root(Arc::new(Resource), ObjectType::Channel, Rights::all())
        .unwrap();
    let fake = insert(&process, fake);
    assert_eq!(receive(&process, fake), -22);
    close(&process, fake);
    let mismatched = {
        let (endpoint, _peer) = Channel::create().unwrap();
        CAP_NODE_ALLOCATOR
            .alloc_root(endpoint, ObjectType::Resource, Rights::all())
            .unwrap()
    };
    let mismatched = insert(&process, mismatched);
    write(&process, IDS, &mismatched.to_le_bytes());
    assert_eq!(send(&process, first, 0, 1), -22);
    close(&process, mismatched);

    // Real task backing can retain endpoint handles indirectly. False leaf tags must fail too.
    let task_backing = Arc::new(Process::new(
        0x4359,
        "payload_cycle",
        ArchAddressSpace::new().unwrap(),
        None,
    ));
    let receiver = {
        let process = process.lock();
        let table = process.handle_table().lock();
        Arc::clone(table.lookup(second).unwrap())
    };
    task_backing.handle_table().lock().insert(receiver).unwrap();
    for tag in [
        ObjectType::Process,
        ObjectType::Job,
        ObjectType::Thread,
        ObjectType::Channel,
        ObjectType::Vmo,
        ObjectType::Resource,
    ] {
        let node = CAP_NODE_ALLOCATOR
            .alloc_root(task_backing.clone(), tag, Rights::TRANSFER)
            .unwrap();
        let handle = insert(&process, Arc::clone(&node));
        write(&process, IDS, &handle.to_le_bytes());
        assert_eq!(send(&process, first, 0, 1), -22);
        assert!(Arc::ptr_eq(
            process.lock().handle_table().lock().lookup(handle).unwrap(),
            &node
        ));
        assert_eq!(receive(&process, second), -11);
        close(&process, handle);
    }
    drop(task_backing);

    let readonly = {
        let process = process.lock();
        let table = process.handle_table().lock();
        table
            .lookup(first)
            .unwrap()
            .attenuate(Rights::READ)
            .unwrap()
    };
    let readonly = insert(&process, readonly);
    assert_eq!(send(&process, readonly, 0, 0), -13);
    close(&process, readonly);
    let writeonly = {
        let process = process.lock();
        let table = process.handle_table().lock();
        table
            .lookup(first)
            .unwrap()
            .attenuate(Rights::WRITE)
            .unwrap()
    };
    let writeonly = insert(&process, writeonly);
    assert_eq!(receive(&process, writeonly), -13);
    close(&process, writeonly);

    for _ in 0..64 {
        assert_eq!(send(&process, first, 0, 0), 0);
    }
    let handle = insert(&process, Arc::clone(&source));
    write(&process, IDS, &handle.to_le_bytes());
    assert_eq!(send(&process, first, 0, 1), -11);
    assert!(process.lock().handle_table().lock().lookup(handle).is_ok());
    assert!(source.is_valid());
    for _ in 0..64 {
        assert_eq!(receive(&process, second), 0);
    }
    assert_eq!(send(&process, first, 0, 1), 0);
    source.revoke();
    assert_eq!(receive(&process, second), 0);
    let received = u32::from_le_bytes(read::<4>(&process, IDS));
    assert!(matches!(
        process.lock().handle_table().lock().lookup(received),
        Err(CapError::Revoked)
    ));
    close(&process, received);
    drop(source);

    table_and_node_exhaustion(&process, first, second);
    assert_eq!(send(&process, first, 0, 0), 0);
    close(&process, first);
    assert_eq!(
        receive(&process, second),
        0,
        "queued data must drain after peer closure"
    );
    assert_eq!(receive(&process, second), -32);
    assert_eq!(send(&process, second, 0, 0), -32);
    let source = resource(Rights::TRANSFER);
    let handle = insert(&process, Arc::clone(&source));
    write(&process, IDS, &handle.to_le_bytes());
    assert_eq!(send(&process, second, 0, 1), -32);
    assert!(Arc::ptr_eq(
        process.lock().handle_table().lock().lookup(handle).unwrap(),
        &source
    ));
    assert!(source.is_valid());
    close(&process, handle);
    drop(source);
    close(&process, second);
    assert_eq!(process.lock().handle_table().lock().count(), 0);
    drop(process);
    drop(backing);
    assert_eq!(
        CAP_NODE_ALLOCATOR.live_count(),
        nodes_before,
        "channel syscalls leaked nodes"
    );
    assert_eq!(
        bitmap::stats().free_pages,
        frames_before,
        "channel syscalls leaked frames"
    );
    kprintln!("[ipc]   channel syscall transactions and user buffers: ok");
}

fn table_and_node_exhaustion(process: &Arc<SpinLock<Process>>, first: u32, second: u32) {
    let source = resource(Rights::TRANSFER | Rights::DUPLICATE);
    let handle = insert(process, Arc::clone(&source));
    write(process, IDS, &handle.to_le_bytes());
    assert_eq!(send(process, first, 0, 1), 0);
    while process
        .lock()
        .handle_table()
        .lock()
        .duplicate(first)
        .is_ok()
    {}
    let before = CAP_NODE_ALLOCATOR.live_count();
    assert_eq!(call(process, SYS_CHAN_CREATE, [BASE, 0, 0, 0, 0, 0]), -24);
    write(process, COUNTS, &[0xc7; 8]);
    assert_eq!(receive(process, second), -24);
    assert_eq!(read::<8>(process, COUNTS), [0xc7; 8]);
    close(process, 255);
    assert_eq!(receive(process, second), 0);
    close(process, 255);
    for handle in 0..255 {
        if handle != first && handle != second {
            close(process, handle);
        }
    }
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), before);
    drop(source);
    let source = resource(Rights::DUPLICATE);
    let mut held = Vec::new();
    loop {
        match source.attenuate(Rights::empty()) {
            Ok(node) => held.push(node),
            Err(CapError::NodeCapacity) => break,
            Err(error) => panic!("unexpected channel quota failure {error:?}"),
        }
    }
    write(process, BASE, &[0xd8; 8]);
    assert_eq!(call(process, SYS_CHAN_CREATE, [BASE, 0, 0, 0, 0, 0]), -28);
    assert_eq!(read::<8>(process, BASE), [0xd8; 8]);
    held.pop();
    assert_eq!(call(process, SYS_CHAN_CREATE, [BASE, 0, 0, 0, 0, 0]), -28);
    assert_eq!(read::<8>(process, BASE), [0xd8; 8]);
    assert_eq!(process.lock().handle_table().lock().count(), 2);
    drop(held);
    drop(source);
}

fn blocking_receive_checks(process: &Arc<SpinLock<Process>>, first: u32, second: u32) {
    let token = Arc::new(WaitToken::new());
    token.start(1);
    let args = [second as usize, DATA, 64, IDS, 64, COUNTS];
    write(process, COUNTS, &[0xc7; 8]);
    let parked = blocking_attempt(process, args, &token, 1);
    assert!(matches!(parked, ReceiveAttempt::Park { node, .. } if node.upgrade().is_some()));
    assert!(!token.is_notified(1));
    assert_eq!(read::<8>(process, COUNTS), [0xc7; 8]);
    assert_eq!(send(process, first, 0, 0), 0);
    assert!(token.is_notified(1));

    let read_only = {
        let process = process.lock();
        let table = process.handle_table().lock();
        Arc::clone(table.lookup(second).unwrap())
    };
    let read_only = read_only.attenuate(Rights::READ).unwrap();
    let denied = insert(process, read_only);
    let mut denied_args = args;
    denied_args[0] = denied as usize;
    assert!(matches!(
        blocking_attempt(process, denied_args, &token, 1),
        ReceiveAttempt::Complete(-13)
    ));
    assert!(matches!(
        blocking_attempt(process, args, &token, 1),
        ReceiveAttempt::Complete(0)
    ));
    assert_eq!(read::<8>(process, COUNTS), [0; 8]);
    close(process, denied);

    token.start(2);
    assert!(matches!(
        blocking_attempt(process, args, &token, 2),
        ReceiveAttempt::Park { .. }
    ));
    write(process, DATA, &[0xa5]);
    assert_eq!(send(process, first, 1, 0), 0);
    let mut short = args;
    short[2] = 0;
    assert!(matches!(
        blocking_attempt(process, short, &token, 2),
        ReceiveAttempt::Complete(-90)
    ));
    assert_eq!(read::<8>(process, COUNTS), [0; 8]);
    assert!(matches!(
        blocking_attempt(process, args, &token, 2),
        ReceiveAttempt::Complete(0)
    ));
    assert_eq!(read::<1>(process, DATA), [0xa5]);
}

fn wait_many_checks(process: &Arc<SpinLock<Process>>, first: u32, second: u32) {
    let token = Arc::new(WaitToken::new());
    token.start(74);
    let mut bytes = [0; 8];
    bytes[..4].copy_from_slice(&first.to_le_bytes());
    bytes[4..].copy_from_slice(&second.to_le_bytes());
    write(process, IDS, &bytes);
    let args = [IDS, 2, 0, 0, 0, 0];
    let (handles, count) = wait_many_input(process, args).unwrap();
    assert_eq!(&handles[..count], &[first, second]);
    assert!(matches!(
        wait_many_input(process, [IDS, 0, 0, 0, 0, 0]),
        Err(-22)
    ));
    assert!(matches!(
        wait_many_input(process, [IDS, 17, 0, 0, 0, 0]),
        Err(-22)
    ));
    assert!(matches!(
        wait_many_input(process, [IDS, 2, 1, 0, 0, 0]),
        Err(-22)
    ));
    assert!(matches!(
        wait_many_input(process, [0, 2, 0, 0, 0, 0]),
        Err(-14)
    ));
    let attempt = wait_many_attempt(process, &handles[..count], &token, 74);
    match attempt {
        WaitManyAttempt::Park { first, extra } => {
            assert!(first.registration.registered(&token, 74));
            let extra = extra.unwrap();
            assert!(extra.members[0]
                .as_ref()
                .unwrap()
                .registration
                .registered(&token, 74));
            first.registration.unregister(&token, 74);
            for member in extra.members.iter().flatten() {
                member.registration.unregister(&token, 74);
            }
        }
        _ => panic!("empty channels did not register whole waitset"),
    }
    assert_eq!(send(process, first, 0, 0), 0);
    assert!(matches!(
        wait_many_attempt(process, &[first, second], &token, 74),
        WaitManyAttempt::Complete(1)
    ));
    assert!(matches!(
        wait_many_attempt(process, &[second, first], &token, 74),
        WaitManyAttempt::Complete(0)
    ));
    // The ready first member cannot hide an invalid later capability or an alias.
    assert!(matches!(
        wait_many_attempt(process, &[second, 256], &token, 74),
        WaitManyAttempt::Complete(-9)
    ));
    let alias = {
        let process = process.lock();
        let node = Arc::clone(process.handle_table().lock().lookup(second).unwrap());
        node
    };
    let alias = insert(process, alias);
    assert!(matches!(
        wait_many_attempt(process, &[second, alias], &token, 74),
        WaitManyAttempt::Complete(-22)
    ));
    close(process, alias);
    assert_eq!(receive(process, second), 0);
    let denied = {
        let process = process.lock();
        let node = process
            .handle_table()
            .lock()
            .lookup(second)
            .unwrap()
            .attenuate(Rights::READ)
            .unwrap();
        node
    };
    let denied = insert(process, denied);
    assert!(matches!(
        wait_many_attempt(process, &[first, denied], &token, 74),
        WaitManyAttempt::Complete(-13)
    ));
    close(process, denied);
    assert!(
        !token.is_notified(74),
        "rolled-back waitset left a registration"
    );
}
