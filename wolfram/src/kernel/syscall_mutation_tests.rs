//! Mutation checks that need allocator state or more than one handle table.

use super::*;
use crate::kernel::capabilities::allocator::NODE_CAPACITY;
use crate::kernel::capabilities::HandleTable;
use crate::kernel::ipc::channel::{Channel, ChannelError, ChannelMessage};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

const FULL: usize = 0x307;

fn call(process: &Arc<SpinLock<Process>>, nr: usize, handle: usize, argument: usize) -> isize {
    dispatch_for_process(process, nr, [handle, argument, 7, 8, 9, 10])
}

pub(super) fn check(process: &Arc<SpinLock<Process>>, output: usize) {
    let before = CAP_NODE_ALLOCATOR.live_count();
    let invoke = |nr, handle, argument| call(process, nr, handle, argument);
    for (handle, error) in [(1, -13), (2, -116), (256, -9)] {
        assert_eq!(invoke(SYS_CAP_ATTENUATE, handle, usize::MAX), error);
        assert_eq!(invoke(SYS_CAP_REVOKE, handle, 0), error);
    }
    for mask in [1 << 10, 1usize << 32, usize::MAX] {
        assert_eq!(invoke(SYS_CAP_ATTENUATE, 3, mask), -22);
    }
    assert_eq!(invoke(SYS_CAP_ATTENUATE, 3, FULL | 8), -13);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), before);

    let child = invoke(SYS_CAP_ATTENUATE, 3, FULL) as usize;
    assert_eq!(child, 0);
    let sibling = invoke(SYS_CAP_ATTENUATE, 3, 0x104) as usize;
    assert_eq!(sibling, 4);
    let zero = invoke(SYS_CAP_ATTENUATE, 3, 0) as usize;
    assert_eq!(zero, 5);
    {
        let process = process.lock();
        let table = process.handle_table().lock();
        let source = table.lookup(3).unwrap();
        let derived = table.lookup(child as u32).unwrap();
        assert!(
            !Arc::ptr_eq(source, derived),
            "attenuation aliased its source"
        );
        assert!(Arc::ptr_eq(derived.parent().unwrap(), source));
        assert_eq!(source.rights().bits(), FULL as u32);
        assert_eq!(derived.rights(), source.rights());
        assert_eq!(table.inspect(sibling as u32).unwrap().rights, 0x104);
        assert!(table.lookup(zero as u32).unwrap().rights().is_empty());
    }
    assert_eq!(invoke(SYS_CAP_INSPECT, zero, output), -13);
    assert_eq!(invoke(SYS_CAP_ATTENUATE, zero, 0), -13);
    assert_eq!(invoke(SYS_CAP_REVOKE, zero, 0), -13);
    assert_eq!(invoke(SYS_CAP_REVOKE, sibling, 0), -13);
    assert_eq!(invoke(SYS_HANDLE_CLOSE, zero, 0), 0);
    let grandchild = invoke(SYS_CAP_ATTENUATE, child, 0x104) as usize;
    assert_eq!(grandchild, 5);
    let alias = invoke(SYS_HANDLE_DUPLICATE, child, 0) as usize;
    assert_eq!(alias, 6);
    let count = process.lock().handle_table().lock().count();
    assert_eq!(invoke(SYS_CAP_REVOKE, alias, 0), 0);
    assert_eq!(process.lock().handle_table().lock().count(), count);
    for handle in [child, alias, grandchild] {
        assert_eq!(invoke(SYS_CAP_INSPECT, handle, output), -116);
        assert_eq!(invoke(SYS_HANDLE_DUPLICATE, handle, 0), -116);
        assert_eq!(invoke(SYS_CAP_ATTENUATE, handle, FULL), -116);
        assert_eq!(invoke(SYS_CAP_REVOKE, handle, 0), -116);
    }
    assert_eq!(invoke(SYS_CAP_INSPECT, 3, output), 0);
    assert_eq!(invoke(SYS_CAP_INSPECT, sibling, output), 0);
    for handle in [child, sibling, grandchild, alias] {
        assert_eq!(invoke(SYS_HANDLE_CLOSE, handle, 0), 0);
    }

    let parent = invoke(SYS_CAP_ATTENUATE, 3, FULL) as usize;
    let child = invoke(SYS_CAP_ATTENUATE, parent, FULL) as usize;
    let parent_lifetime = {
        let process = process.lock();
        let table = process.handle_table().lock();
        Arc::downgrade(table.lookup(parent as u32).unwrap())
    };
    assert_eq!(invoke(SYS_HANDLE_CLOSE, parent, 0), 0);
    assert!(
        parent_lifetime.upgrade().is_some(),
        "child lost its parent owner"
    );
    assert_eq!(invoke(SYS_CAP_INSPECT, child, output), 0);
    assert_eq!(invoke(SYS_HANDLE_CLOSE, child, 0), 0);
    assert!(
        parent_lifetime.upgrade().is_none(),
        "ancestry was not reclaimed"
    );
    drop(parent_lifetime);

    for _ in 0..2048 {
        let child = invoke(SYS_CAP_ATTENUATE, 3, FULL);
        assert_eq!(child, 0);
        assert_eq!(invoke(SYS_HANDLE_CLOSE, child as usize, 0), 0);
    }
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), before);

    let source = {
        let process = process.lock();
        let table = process.handle_table().lock();
        Arc::clone(table.lookup(3).unwrap())
    };
    let mut held = Vec::new();
    loop {
        match source.attenuate(source.rights()) {
            Ok(child) => held.push(child),
            Err(CapError::NodeCapacity) => break,
            Err(error) => panic!("unexpected node allocation failure: {error:?}"),
        }
    }
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), NODE_CAPACITY);
    assert_eq!(invoke(SYS_CAP_ATTENUATE, 3, FULL), -28);
    assert_eq!(process.lock().handle_table().lock().count(), 3);
    while invoke(SYS_HANDLE_DUPLICATE, 3, 0) >= 0 {}
    assert_eq!(process.lock().handle_table().lock().count(), 256);
    assert_eq!(invoke(SYS_CAP_ATTENUATE, 3, FULL), -24);
    assert_eq!(invoke(SYS_CAP_ATTENUATE, 3, 1 << 10), -22);
    assert_eq!(invoke(SYS_CAP_ATTENUATE, 1, FULL), -13);
    assert_eq!(invoke(SYS_CAP_ATTENUATE, 2, FULL), -116);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), NODE_CAPACITY);
    for handle in 0..256 {
        if !matches!(handle, 1..=3) {
            assert_eq!(invoke(SYS_HANDLE_CLOSE, handle, 0), 0);
        }
    }
    drop(held.pop().expect("quota should contain children"));
    assert_eq!(invoke(SYS_CAP_ATTENUATE, 3, FULL), 0);
    assert_eq!(invoke(SYS_HANDLE_CLOSE, 0, 0), 0);
    drop(held);
    drop(source);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), before);
    close_without_locks(process);
    rejected_send_without_locks();
    ownership_checks();
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), before);
    assert_eq!(process.lock().handle_table().lock().count(), 3);
    kprintln!("[cap]   attenuation, subtree revocation, and ownership: ok");
}

fn rejected_send_without_locks() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct ReentrantObject {
        endpoint: alloc::sync::Weak<Channel>,
    }
    impl KernelObject for ReentrantObject {
        fn type_name(&self) -> &'static str {
            "Resource"
        }
    }
    impl Drop for ReentrantObject {
        fn drop(&mut self) {
            let endpoint = self
                .endpoint
                .upgrade()
                .expect("rejected send lost endpoint");
            let _ = endpoint.signals();
            DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let before = DROPS.load(Ordering::Relaxed);
    let (sender, _receiver) = Channel::create_with_limits(0, 64, 4).unwrap();
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(
            Arc::new(ReentrantObject {
                endpoint: Arc::downgrade(&sender),
            }),
            ObjectType::Resource,
            Rights::all(),
        )
        .unwrap();
    let message = ChannelMessage::new(Vec::new(), vec![node]);
    let rejected = { sender.send(message) };
    let (error, message) = rejected.unwrap_err();
    assert_eq!(error, ChannelError::QueueFull);
    assert_eq!(DROPS.load(Ordering::Relaxed), before);
    assert!(message.handles[0].is_valid());
    drop(message);
    assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);

    let root = CAP_NODE_ALLOCATOR
        .alloc_root(
            Arc::new(crate::kernel::capabilities::Resource),
            ObjectType::Resource,
            Rights::all(),
        )
        .unwrap();
    let branch = root.attenuate(Rights::all()).unwrap();
    let descendant = branch.attenuate(Rights::READ).unwrap();
    let alias = Arc::clone(&branch);
    let rejected = { sender.send(ChannelMessage::new(Vec::new(), vec![branch])) };
    let (error, message) = rejected.unwrap_err();
    assert_eq!(error, ChannelError::QueueFull);
    assert!(alias.is_valid() && descendant.is_valid());
    assert!(root.is_valid());
    drop(message);
}

fn close_without_locks(process: &Arc<SpinLock<Process>>) {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct ReentrantObject {
        process: alloc::sync::Weak<SpinLock<Process>>,
    }
    impl KernelObject for ReentrantObject {
        fn type_name(&self) -> &'static str {
            "Resource"
        }
    }
    impl Drop for ReentrantObject {
        fn drop(&mut self) {
            let process = self.process.upgrade().expect("close lost current process");
            let process = process
                .try_lock()
                .expect("object dropped under process lock");
            let _table = process
                .handle_table()
                .try_lock()
                .expect("object dropped under table lock");
            DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let before = DROPS.load(Ordering::Relaxed);
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(
            Arc::new(ReentrantObject {
                process: Arc::downgrade(process),
            }),
            ObjectType::Resource,
            Rights::READ,
        )
        .unwrap();
    let handle = {
        let process = process.lock();
        let mut table = process.handle_table().lock();
        table.insert(node).unwrap()
    };
    assert_eq!(call(process, SYS_HANDLE_CLOSE, handle as usize, 0), 0);
    assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);
}

struct TrackedObject(&'static AtomicUsize);

impl KernelObject for TrackedObject {
    fn type_name(&self) -> &'static str {
        "Resource"
    }
}

impl Drop for TrackedObject {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn ownership_checks() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let drops_before = DROPS.load(Ordering::Relaxed);
    let nodes_before = CAP_NODE_ALLOCATOR.live_count();
    let rights = Rights::all();
    let root = CAP_NODE_ALLOCATOR
        .alloc_root(
            Arc::new(TrackedObject(&DROPS)),
            ObjectType::Resource,
            rights,
        )
        .unwrap();
    let parent = root.attenuate(rights).unwrap();
    let child = parent.attenuate(rights).unwrap();
    let sibling = root.attenuate(rights).unwrap();
    let mut sender = HandleTable::new();
    let mut receiver = HandleTable::new();
    let handle = sender.insert(child).unwrap();
    let moved = sender.transfer_send(handle).unwrap();
    assert_eq!(sender.count(), 0);
    let mut message = ChannelMessage::new(Vec::new(), vec![moved]);
    receiver.insert(Arc::clone(&root)).unwrap();
    for _ in 1..256 {
        receiver.duplicate(0).unwrap();
    }
    let moved = message.handles.pop().unwrap();
    let (error, returned) = receiver.transfer_recv(moved).unwrap_err();
    assert_eq!(error, CapError::TableFull);
    message.handles.push(returned);
    assert_eq!(message.handles.len(), 1);
    assert!(
        message.handles[0].is_valid(),
        "failed receipt revoked the capability"
    );
    drop(receiver.remove(255).unwrap());
    let received = receiver
        .transfer_recv(message.handles.pop().unwrap())
        .unwrap();
    drop(message);
    assert!(receiver.lookup(received).is_ok());
    parent.revoke();
    assert_eq!(receiver.lookup(received).unwrap_err(), CapError::Revoked);
    assert!(root.is_valid() && sibling.is_valid());
    drop(receiver);
    drop(sender);
    drop(parent);
    drop(sibling);

    let branch = root.attenuate(rights).unwrap();
    let grandchild = branch.attenuate(rights).unwrap();
    drop(ChannelMessage::new(Vec::new(), vec![Arc::clone(&branch)]));
    assert!(branch.is_valid() && grandchild.is_valid());
    assert!(root.is_valid());
    drop(grandchild);
    drop(branch);
    drop(root);
    assert_eq!(DROPS.load(Ordering::Relaxed), drops_before + 1);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes_before);

    let mut leaf = CAP_NODE_ALLOCATOR
        .alloc_root(
            Arc::new(TrackedObject(&DROPS)),
            ObjectType::Resource,
            rights,
        )
        .unwrap();
    let mut depth = 1;
    loop {
        match leaf.attenuate(rights) {
            Ok(child) => {
                leaf = child;
                depth += 1;
            }
            Err(CapError::NodeCapacity) => break,
            Err(error) => panic!("unexpected chain allocation failure: {error:?}"),
        }
    }
    assert_eq!(depth, NODE_CAPACITY - nodes_before);
    assert!(leaf.is_valid());
    drop(leaf);
    assert_eq!(DROPS.load(Ordering::Relaxed), drops_before + 2);
    assert_eq!(
        CAP_NODE_ALLOCATOR.live_count(),
        nodes_before,
        "deep chain leaked quota"
    );
}
