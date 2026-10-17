//! Resident test fixtures and observations, kept outside runtime syscall execution.

use super::*;
use crate::kernel::capabilities::{CapNode, KernelObject, ObjectType, Rights, CAP_NODE_ALLOCATOR};

#[path = "syscall_mutation_tests.rs"]
mod mutation_tests;

const CANARIES: [usize; 6] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
const OUTCOMES: [(usize, isize); 24] = [
    (SYS_HANDLE_CLOSE, 0),
    (SYS_HANDLE_CLOSE, -9),
    (SYS_HANDLE_DUPLICATE, 0),
    (SYS_HANDLE_DUPLICATE, -9),
    (SYS_HANDLE_DUPLICATE, -13),
    (SYS_HANDLE_DUPLICATE, -116),
    (SYS_HANDLE_DUPLICATE, -24),
    (SYS_CAP_INSPECT, 0),
    (SYS_CAP_INSPECT, -9),
    (SYS_CAP_INSPECT, -13),
    (SYS_CAP_INSPECT, -116),
    (SYS_CAP_INSPECT, -14),
    (SYS_YIELD, 0),
    (99, -38),
    (SYS_CAP_ATTENUATE, 0),
    (SYS_CAP_ATTENUATE, -9),
    (SYS_CAP_ATTENUATE, -13),
    (SYS_CAP_ATTENUATE, -22),
    (SYS_CAP_ATTENUATE, -116),
    (SYS_CAP_ATTENUATE, -24),
    (SYS_CAP_REVOKE, 0),
    (SYS_CAP_REVOKE, -9),
    (SYS_CAP_REVOKE, -13),
    (SYS_CAP_REVOKE, -116),
];
const _: () = assert!(OUTCOMES.len() < u32::BITS as usize);
const CHANNEL_OUTCOMES: [(usize, isize); 13] = [
    (SYS_CHAN_CREATE, 0),
    (SYS_CHAN_CREATE, -14),
    (SYS_CHAN_SEND, 0),
    (SYS_CHAN_SEND, -14),
    (SYS_CHAN_SEND, -22),
    (SYS_CHAN_SEND, -13),
    (SYS_CHAN_SEND, -32),
    (SYS_CHAN_RECV, 0),
    (SYS_CHAN_RECV, -11),
    (SYS_CHAN_RECV, -90),
    (SYS_CHAN_RECV, -14),
    (SYS_CHAN_RECV, -13),
    (SYS_CHAN_RECV, -32),
];

pub(super) struct Observer {
    calls: usize,
    valid: bool,
    outcomes: u32,
    channel_outcomes: u32,
}

impl Observer {
    pub(super) fn new() -> Self {
        Self {
            calls: 0,
            valid: true,
            outcomes: 0,
            channel_outcomes: 0,
        }
    }

    pub(super) fn observe(&mut self, nr: usize, args: [usize; 6], result: isize, isolation: bool) {
        self.valid &= !isolation;
        if self.calls < 3 {
            self.valid &= args == CANARIES
                && if self.calls < 2 {
                    nr == SYS_YIELD && result == 0
                } else {
                    nr == 99 && result == -38
                };
        } else {
            let normalized =
                if matches!(nr, SYS_HANDLE_DUPLICATE | SYS_CAP_ATTENUATE) && result >= 0 {
                    0
                } else {
                    result
                };
            for (index, &(expected_nr, expected_result)) in CHANNEL_OUTCOMES.iter().enumerate() {
                if nr == expected_nr && result == expected_result {
                    self.channel_outcomes |= 1 << index;
                }
            }
            let observed_nr = if nr > SYS_CHAN_RECV { 99 } else { nr };
            for (index, &(expected_nr, expected_result)) in OUTCOMES.iter().enumerate() {
                if observed_nr == expected_nr && normalized == expected_result {
                    self.outcomes |= 1 << index;
                }
            }
        }
        self.calls = self.calls.saturating_add(1);
    }

    #[cfg(target_arch = "x86_64")]
    pub(super) fn x86_flags(&self, nr: usize, flags: u64) {
        if cfg!(feature = "qemu-debug-port") && self.calls < 3 && matches!(nr, SYS_YIELD | 99) {
            assert_eq!(flags, 0xcd7, "user flags probe was not armed");
        }
    }

    pub(super) fn finish(&self, status: usize, handles: usize, isolation: bool) {
        assert_eq!(status, 0, "user probe failed");
        assert!(
            self.valid && !isolation && self.calls >= 3,
            "user roundtrip corrupted"
        );
        assert_eq!(
            self.outcomes,
            (1u32 << OUTCOMES.len()) - 1,
            "incomplete capability syscall coverage"
        );
        assert_eq!(
            self.channel_outcomes,
            (1u32 << CHANNEL_OUTCOMES.len()) - 1,
            "incomplete channel syscall coverage"
        );
        assert_eq!(handles, 0, "user probe leaked handles");
    }
}

pub(super) struct Fixtures {
    nodes: [Arc<CapNode>; 3],
}

impl Fixtures {
    pub(super) fn delegate(process: &Process) -> Self {
        let object: Arc<dyn KernelObject> = Arc::new(Vmo::create(4096).expect("fixture VMO"));
        let full =
            Rights::DUPLICATE | Rights::TRANSFER | Rights::INSPECT | Rights::READ | Rights::MANAGE;
        let nodes = [full, Rights::READ, full].map(|rights| {
            CAP_NODE_ALLOCATOR
                .alloc_root(Arc::clone(&object), ObjectType::Vmo, rights)
                .expect("fixture capability")
        });
        nodes[2].revoke();
        let mut table = process.handle_table().lock();
        assert_eq!(table.count(), 0, "fixture delegation requires empty table");
        for (slot, node) in nodes.iter().enumerate() {
            assert_eq!(
                table
                    .insert(Arc::clone(node))
                    .unwrap_or_else(|(error, _)| panic!("fixture insert: {error:?}")),
                slot as u32
            );
        }
        Self { nodes }
    }
}

pub(super) fn self_test() {
    crate::kernel::capabilities::self_test();
    const BASE: usize = 0x30_0000_0000;
    let before = bitmap::stats().free_pages;
    let mut backing = Vmo::create(8192).expect("syscall copy backing");
    let process = Arc::new(SpinLock::new(Process::new(
        0,
        "syscall_self_test",
        ArchAddressSpace::new().unwrap(),
        None,
    )));
    let fixtures = Fixtures::delegate(&process.lock());
    unsafe {
        process
            .lock()
            .address_space_mut()
            .map_vmo(BASE, backing.frames(), MapFlags::user_rw())
            .unwrap();
    }
    let call =
        |nr, handle, pointer| dispatch_for_process(&process, nr, [handle, pointer, 7, 8, 9, 10]);
    assert_eq!(call(SYS_YIELD, usize::MAX, 0), 0);
    assert_eq!(call(99, 0, 0), -38);
    for nr in [
        SYS_HANDLE_CLOSE,
        SYS_HANDLE_DUPLICATE,
        SYS_CAP_INSPECT,
        SYS_CAP_ATTENUATE,
        SYS_CAP_REVOKE,
    ] {
        assert_eq!(
            call(nr, 1usize << 32, BASE),
            -9,
            "high handle bits truncated"
        );
    }
    assert_eq!(call(SYS_CAP_INSPECT, 0, BASE + 4092), 0);
    let mut bytes = [0; 8];
    backing.read(4092, &mut bytes).unwrap();
    assert_eq!(bytes, [2, 0, 0, 0, 7, 3, 0, 0]);
    for (handle, error) in [(1, -13), (2, -116), (256, -9)] {
        assert_eq!(
            call(SYS_CAP_INSPECT, handle, 0),
            error,
            "handle validation must precede pointer validation"
        );
    }
    for pointer in [0, usize::MAX, BASE + 8192, arch::isolation_address()] {
        assert_eq!(call(SYS_CAP_INSPECT, 0, pointer), -14);
    }
    backing.write(4092, &[0xa5; 8]).unwrap();
    for flags in [MapFlags::user_ro(), MapFlags::kernel_rw()] {
        unsafe {
            process
                .lock()
                .address_space_mut()
                .protect_page(BASE + 4096, flags)
                .unwrap();
        }
        assert_eq!(call(SYS_CAP_INSPECT, 0, BASE + 4092), -14);
        backing.read(4092, &mut bytes).unwrap();
        assert_eq!(bytes, [0xa5; 8], "failed inspect partially wrote output");
    }
    unsafe {
        process
            .lock()
            .address_space_mut()
            .unmap_page(BASE + 4096)
            .unwrap();
    }
    assert_eq!(call(SYS_CAP_INSPECT, 0, BASE + 4092), -14);
    backing.read(4092, &mut bytes).unwrap();
    assert_eq!(bytes, [0xa5; 8]);
    assert_eq!(call(SYS_HANDLE_DUPLICATE, 0, 0), 3);
    assert_eq!(call(SYS_HANDLE_CLOSE, 0, 0), 0);
    assert_eq!(call(SYS_HANDLE_CLOSE, 0, 0), -9);
    assert_eq!(
        call(SYS_CAP_INSPECT, 3, BASE),
        0,
        "closing source revoked alias"
    );
    for _ in 0..2048 {
        let handle = call(SYS_HANDLE_DUPLICATE, 3, 0);
        assert_eq!(handle, 0);
        assert_eq!(call(SYS_HANDLE_CLOSE, handle as usize, 0), 0);
    }
    mutation_tests::check(&process, BASE);
    loop {
        let result = call(SYS_HANDLE_DUPLICATE, 3, 0);
        if result == -24 {
            break;
        }
        assert!(result >= 0);
    }
    assert_eq!(process.lock().handle_table().lock().count(), 256);
    let nodes_before = CAP_NODE_ALLOCATOR.live_count();
    assert_eq!(call(SYS_CAP_ATTENUATE, 3, 0x307), -24);
    assert_eq!(
        CAP_NODE_ALLOCATOR.live_count(),
        nodes_before,
        "full table allocated a child"
    );
    for (handle, error) in [(1, -13), (2, -116), (256, -9)] {
        assert_eq!(
            call(SYS_HANDLE_DUPLICATE, handle, 0),
            error,
            "source validation must precede capacity check"
        );
    }
    assert_eq!(process.lock().handle_table().lock().count(), 256);
    assert_eq!(call(SYS_HANDLE_CLOSE, 0, 0), 0);
    assert_eq!(call(SYS_HANDLE_DUPLICATE, 3, 0), 0);
    fixtures.nodes[0].revoke();
    for handle in [0, 3] {
        assert_eq!(call(SYS_CAP_INSPECT, handle, BASE), -116);
        assert_eq!(call(SYS_HANDLE_DUPLICATE, handle, 0), -116);
    }
    for handle in 0..256 {
        assert_eq!(
            call(SYS_HANDLE_CLOSE, handle, 0),
            0,
            "revoked slots must be closeable"
        );
    }
    assert_eq!(process.lock().handle_table().lock().count(), 0);
    drop(process);
    drop(fixtures);
    drop(backing);
    assert_eq!(
        bitmap::stats().free_pages,
        before,
        "syscall tests leaked frames"
    );
    kprintln!("[cap]   handle syscall validation and copy checks: ok");
}
