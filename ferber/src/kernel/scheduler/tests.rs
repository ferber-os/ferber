//! Exercise production reconciliation and reaping without switching test stacks.

use super::*;
use crate::kernel::capabilities::{ObjectType, Rights, CAP_NODE_ALLOCATOR};
use crate::kernel::ipc::channel::{Channel, ChannelMessage};
use crate::kernel::syscall::{dispatch_for_process, SYS_HANDLE_CLOSE};
use crate::kernel::task::Job;

struct Fixture {
    process: Arc<SpinLock<Process>>,
    thread: Arc<SpinLock<Thread>>,
    endpoint: Arc<Channel>,
    peer: Arc<Channel>,
    node: Arc<CapNode>,
    handle: u32,
    index: usize,
}

impl Fixture {
    fn new(root: &Arc<SpinLock<Job>>) -> Self {
        const ENTRY: usize = 0x20_0040;
        let process = root.lock().create_process("scheduler_test").unwrap();
        process.lock().set_state(ProcessState::Running);
        let thread = process
            .lock()
            .create_named_thread("scheduler_test/main", ENTRY, 4096)
            .unwrap();
        let (endpoint, peer) = Channel::create().unwrap();
        let node = CAP_NODE_ALLOCATOR
            .alloc_root(
                endpoint.clone(),
                ObjectType::Channel,
                Rights::READ | Rights::WRITE | Rights::WAIT | Rights::DUPLICATE | Rights::MANAGE,
            )
            .unwrap();
        let handle = process
            .lock()
            .handle_table()
            .lock()
            .insert(node.clone())
            .unwrap();
        admit(process.clone(), thread.clone()).unwrap();
        assert_eq!(
            thread.lock().user_entry(),
            ENTRY,
            "admission lost user entry"
        );
        let index = RUNTIME
            .lock()
            .tasks
            .iter()
            .position(|task| {
                task.as_ref()
                    .is_some_and(|task| Arc::ptr_eq(&task.thread, &thread))
            })
            .unwrap();
        Self {
            process,
            thread,
            endpoint,
            peer,
            node,
            handle,
            index,
        }
    }

    fn arm(&self) -> (Arc<WaitToken>, u64) {
        {
            let mut runtime = RUNTIME.lock();
            assert!(runtime.current.is_none());
            runtime.ready.retain(|&index| index != self.index);
            runtime.current = Some(self.index);
        }
        self.thread.lock().set_state(ThreadState::Running);
        let (token, generation) = prepare_wait().unwrap();
        assert!(self.endpoint.register_wait(&token, generation).unwrap());
        let process_id = self.process.lock().id();
        {
            let mut runtime = RUNTIME.lock();
            runtime.tasks[self.index].as_mut().unwrap().pending = Some(Pending {
                process_id,
                handle: self.handle as usize,
                node: Arc::downgrade(&self.node),
                registration: self.endpoint.wait_registration(),
                generation,
                error: None,
                extra: None,
            });
            runtime.current = None;
        }
        self.thread.lock().set_state(ThreadState::Blocked);
        (token, generation)
    }

    fn ready_occurrences(&self) -> usize {
        RUNTIME
            .lock()
            .ready
            .iter()
            .filter(|&&index| index == self.index)
            .count()
    }

    fn error(&self) -> Option<isize> {
        RUNTIME.lock().tasks[self.index]
            .as_ref()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .error
    }

    fn dispose(self) {
        self.thread.lock().set_state(ThreadState::Dead);
        assert!(RUNTIME.lock().current.is_none());
        reap();
        let process = Arc::downgrade(&self.process);
        let thread = Arc::downgrade(&self.thread);
        drop(self);
        assert!(process.upgrade().is_none(), "reaped process retained");
        assert!(thread.upgrade().is_none(), "reaped thread retained");
    }
}

fn notified_close_reuse(root: &Arc<SpinLock<Job>>) {
    let fixture = Fixture::new(root);
    let (token, generation) = fixture.arm();
    // Notification arrives after registration but before the task could park.
    fixture.thread.lock().set_state(ThreadState::Running);
    fixture
        .peer
        .send(ChannelMessage::new(Vec::new(), Vec::new()))
        .unwrap();
    assert!(token.is_notified(generation));
    fixture.thread.lock().set_state(ThreadState::Blocked);
    reconcile();
    reconcile();
    assert_eq!(fixture.thread.lock().state(), ThreadState::Ready);
    assert_eq!(fixture.ready_occurrences(), 1);
    assert_eq!(fixture.error(), None);

    assert_eq!(
        dispatch_for_process(
            &fixture.process,
            SYS_HANDLE_CLOSE,
            [fixture.handle as usize, 0, 0, 0, 0, 0],
        ),
        0
    );
    let replacement = fixture
        .process
        .lock()
        .handle_table()
        .lock()
        .insert(fixture.node.clone())
        .unwrap();
    assert_eq!(replacement, fixture.handle);
    reconcile();
    reconcile();
    assert_eq!(fixture.error(), Some(-9), "same-node slot reuse hid close");
    assert_eq!(fixture.ready_occurrences(), 1);
    drop(token);
    fixture.dispose();
}

fn wait_many_cancellation(root: &Arc<SpinLock<Job>>) {
    for revoked in [false, true] {
        let fixture = Fixture::new(root);
        let (extra_endpoint, _extra_peer) = Channel::create().unwrap();
        let extra_node = CAP_NODE_ALLOCATOR
            .alloc_root(
                extra_endpoint.clone(),
                ObjectType::Channel,
                Rights::READ | Rights::WAIT | Rights::MANAGE,
            )
            .unwrap();
        let extra_handle = fixture
            .process
            .lock()
            .handle_table()
            .lock()
            .insert(extra_node.clone())
            .unwrap();
        let (token, generation) = fixture.arm();
        assert!(extra_endpoint.register_wait(&token, generation).unwrap());
        {
            let mut runtime = RUNTIME.lock();
            runtime.tasks[fixture.index]
                .as_mut()
                .unwrap()
                .pending
                .as_mut()
                .unwrap()
                .extra = Some(Arc::new(ExtraWaits {
                members: core::array::from_fn(|index| {
                    (index == 0).then(|| WaitMember {
                        handle: extra_handle as usize,
                        node: Arc::downgrade(&extra_node),
                        registration: extra_endpoint.wait_registration(),
                    })
                }),
            }));
        }
        if revoked {
            extra_node.revoke();
        } else {
            // A ready first endpoint must not hide close/reuse of a later watched slot.
            fixture
                .peer
                .send(ChannelMessage::new(Vec::new(), Vec::new()))
                .unwrap();
            assert_eq!(
                dispatch_for_process(
                    &fixture.process,
                    SYS_HANDLE_CLOSE,
                    [extra_handle as usize, 0, 0, 0, 0, 0]
                ),
                0
            );
            assert_eq!(
                fixture
                    .process
                    .lock()
                    .handle_table()
                    .lock()
                    .insert(extra_node.clone())
                    .unwrap(),
                extra_handle
            );
        }
        reconcile();
        assert_eq!(fixture.error(), Some(if revoked { -116 } else { -9 }));
        assert_eq!(fixture.ready_occurrences(), 1);
        assert!(!fixture
            .endpoint
            .wait_registration()
            .registered(&token, generation));
        assert!(!extra_endpoint
            .wait_registration()
            .registered(&token, generation));
        drop(extra_node);
        drop(extra_endpoint);
        drop(token);
        fixture.dispose();
    }
}

fn ancestor_revocation(root: &Arc<SpinLock<Job>>) {
    let mut fixture = Fixture::new(root);
    let ancestor = fixture.node.clone();
    let child = ancestor.attenuate(Rights::READ | Rights::WAIT).unwrap();
    {
        let process = fixture.process.lock();
        let mut table = process.handle_table().lock();
        drop(table.remove(fixture.handle).unwrap());
        assert_eq!(table.insert(child.clone()).unwrap(), fixture.handle);
    }
    fixture.node = child;
    let (token, generation) = fixture.arm();
    ancestor.revoke();
    reconcile();
    assert_eq!(fixture.error(), Some(-116));
    assert!(token.is_notified(generation));
    assert_eq!(fixture.ready_occurrences(), 1);
    drop(token);
    drop(ancestor);
    fixture.dispose();
}

fn killed_waiter(root: &Arc<SpinLock<Job>>) {
    let fixture = Fixture::new(root);
    let (token, generation) = fixture.arm();
    fixture.process.lock().kill();
    assert_eq!(
        suspend_state(&fixture.thread, ThreadState::Ready),
        ThreadState::Dead,
        "yield resurrected a killed thread"
    );
    reap();
    assert_eq!(fixture.ready_occurrences(), 0);
    assert!(RUNTIME.lock().tasks[fixture.index].is_none());
    fixture
        .peer
        .send(ChannelMessage::new(Vec::new(), Vec::new()))
        .unwrap();
    assert!(
        !token.is_notified(generation),
        "reaped waiter remained registered"
    );
    reconcile();
    assert_eq!(fixture.ready_occurrences(), 0, "dead waiter resurrected");
    drop(token);
    fixture.dispose();
}

fn sibling_thread_lifetime(root: &Arc<SpinLock<Job>>) {
    let fixture = Fixture::new(root);
    let sibling = fixture
        .process
        .lock()
        .create_named_thread("scheduler_test/sibling", 0, 4096)
        .unwrap();
    admit(fixture.process.clone(), sibling.clone()).unwrap();
    fixture.thread.lock().set_state(ThreadState::Dead);
    reap();
    assert_eq!(fixture.process.lock().thread_count(), 1);
    assert_eq!(fixture.process.lock().state(), ProcessState::Running);
    assert_eq!(fixture.process.lock().handle_table().lock().count(), 1);
    assert_eq!(sibling.lock().state(), ThreadState::Ready);
    let weak = Arc::downgrade(&sibling);
    sibling.lock().set_state(ThreadState::Dead);
    drop(sibling);
    reap();
    assert!(weak.upgrade().is_none());
    assert_eq!(fixture.process.lock().state(), ProcessState::Dead);
    assert_eq!(fixture.process.lock().handle_table().lock().count(), 0);
    fixture.dispose();
}

fn failed_task_construction(root: &Arc<SpinLock<Job>>) {
    let process = root.lock().create_process("failed_task_test").unwrap();
    {
        let mut process = process.lock();
        assert_eq!(
            process.reserve_backing(usize::MAX),
            Err(TaskError::OutOfMemory)
        );
        assert!(matches!(
            process.create_named_thread("failed_task_test/main", 0x20_0040, 0),
            Err(TaskError::OutOfMemory)
        ));
        assert_eq!(process.thread_count(), 0);
        assert_eq!(process.handle_table().lock().count(), 0);
    }
    let weak = Arc::downgrade(&process);
    let id = process.lock().id();
    drop(root.lock().detach_process(id).unwrap());
    drop(process);
    assert!(
        weak.upgrade().is_none(),
        "failed construction retained process"
    );
}

fn startup_stack_pointer_bounds(root: &Arc<SpinLock<Job>>) {
    let process = root.lock().create_process("startup_stack_test").unwrap();
    let thread = process
        .lock()
        .create_named_thread("startup_stack_test/main", 0x20_0040, 4096)
        .unwrap();
    {
        let mut thread = thread.lock();
        assert_eq!(thread.user_initial_sp(), None);
        assert_eq!(thread.set_user_initial_sp(0x1000), Err(()));
        thread.set_user_stack(Vmo::create(4096).unwrap());
        thread.set_user_stack_mapping(0x1000);
        assert_eq!(
            thread.user_initial_sp(),
            Some(0x2000),
            "resident stack default"
        );
        for invalid in [0xff0, 0x1000, 0x1001, 0x1fff, 0x2001, 0x2010] {
            assert_eq!(thread.set_user_initial_sp(invalid), Err(()));
            assert_eq!(thread.user_initial_sp(), Some(0x2000));
        }
        for valid in [0x1010, 0x1fe0, 0x2000] {
            assert_eq!(thread.set_user_initial_sp(valid), Ok(()));
            assert_eq!(thread.user_initial_sp(), Some(valid));
            assert_eq!(thread.user_stack_mapping(), Some((0x1000, 4096)));
        }
        thread.set_user_stack_mapping(usize::MAX - 4095);
        assert_eq!(thread.user_initial_sp(), None);
        assert_eq!(thread.set_user_initial_sp(usize::MAX - 4095), Err(()));
    }
    let id = process.lock().id();
    process.lock().kill();
    drop(root.lock().detach_process(id).unwrap());
}

#[cfg(feature = "init")]
fn init_wait_contract(root: &Arc<SpinLock<Job>>) {
    let fixture = Fixture::new(root);
    let (token, generation) = fixture.arm();
    let mut session = crate::kernel::init::Session::test_session(
        crate::kernel::elf_load::Admitted {
            process_id: fixture.process.lock().id(),
            thread_id: fixture.thread.lock().id(),
        },
        Arc::downgrade(&fixture.node),
        Arc::clone(&fixture.peer),
    );
    assert!(init_wait_matches(&session));
    fixture.endpoint.unregister_wait(&token, generation);
    assert!(!init_wait_matches(&session), "unregistered wait accepted");
    assert!(fixture.endpoint.register_wait(&token, generation).unwrap());
    assert!(init_wait_matches(&session));
    session.admitted.thread_id += 1;
    assert!(!init_wait_matches(&session));
    session.admitted.thread_id -= 1;
    session.admitted.process_id += 1;
    assert!(!init_wait_matches(&session));
    session.admitted.process_id -= 1;
    // The same pair's same side is not an external peer, nor is an unrelated pair.
    session.peer = Some(Arc::clone(&fixture.endpoint));
    assert!(!init_wait_matches(&session));
    let (other, _) = Channel::create().unwrap();
    session.peer = Some(other);
    assert!(!init_wait_matches(&session));
    session.peer = Some(Arc::clone(&fixture.peer));
    {
        let mut runtime = RUNTIME.lock();
        let pending = runtime.tasks[fixture.index]
            .as_mut()
            .unwrap()
            .pending
            .as_mut()
            .unwrap();
        pending.generation += 1;
    }
    assert!(!init_wait_matches(&session));
    {
        let mut runtime = RUNTIME.lock();
        let pending = runtime.tasks[fixture.index]
            .as_mut()
            .unwrap()
            .pending
            .as_mut()
            .unwrap();
        pending.generation -= 1;
        pending.error = Some(-9);
    }
    assert!(!init_wait_matches(&session));
    RUNTIME.lock().tasks[fixture.index]
        .as_mut()
        .unwrap()
        .pending
        .as_mut()
        .unwrap()
        .error = None;
    assert!(init_wait_matches(&session));
    let second = Fixture::new(root);
    second.arm();
    assert!(
        init_wait_matches(&session),
        "init identity depends on task count"
    );
    second.dispose();
    assert!(init_wait_matches(&session));
    token.notify(generation);
    assert!(!init_wait_matches(&session));
    reconcile();
    assert_eq!(fixture.thread.lock().state(), ThreadState::Ready);
    assert!(!init_wait_matches(&session));
    drop(session);
    fixture.dispose();

    let fixture = Fixture::new(root);
    fixture.arm();
    let session = crate::kernel::init::Session::test_session(
        crate::kernel::elf_load::Admitted {
            process_id: fixture.process.lock().id(),
            thread_id: fixture.thread.lock().id(),
        },
        Arc::downgrade(&fixture.node),
        Arc::clone(&fixture.peer),
    );
    fixture.node.revoke();
    assert!(!init_wait_matches(&session), "revoked init wait accepted");
    reconcile();
    assert_eq!(fixture.error(), Some(-116));
    assert_eq!(fixture.thread.lock().state(), ThreadState::Ready);
    drop(session);
    fixture.dispose();
}

fn rooted_wait_graph(root: &Arc<SpinLock<Job>>) {
    let first = Fixture::new(root);
    first.arm();
    let second = Fixture::new(root);
    second.arm();
    assert!(!idle_quiescent());
    let mut source = register_source(&first.peer, SourceKind::TerminalBootstrap).unwrap();
    assert!(
        !idle_quiescent(),
        "unrelated external source concealed orphan wait"
    );
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(second.peer.clone(), ObjectType::Channel, Rights::WRITE)
        .unwrap();
    first
        .process
        .lock()
        .handle_table()
        .lock()
        .insert(Arc::clone(&node))
        .unwrap();
    assert!(
        idle_quiescent(),
        "producer chain did not reach external root"
    );
    source.cancel();
    assert!(
        !idle_quiescent(),
        "canceled source concealed all-blocked tasks"
    );
    let replacement = register_source(&first.peer, SourceKind::Software).unwrap();
    source.cancel();
    assert!(
        idle_quiescent(),
        "stale cancellation removed replacement source"
    );
    node.revoke();
    assert!(
        !idle_quiescent(),
        "revoked WRITE authority supplied a producer edge"
    );
    drop(replacement);
    let wrong_side = register_source(&first.endpoint, SourceKind::TerminalBootstrap).unwrap();
    assert!(
        !idle_quiescent(),
        "same endpoint side accepted as a producer"
    );
    drop(wrong_side);
    drop(source);
    drop(node);
    second.dispose();
    first.dispose();
}

fn admission_while_running(root: &Arc<SpinLock<Job>>) {
    let parent = Fixture::new(root);
    {
        let mut runtime = RUNTIME.lock();
        runtime.ready.retain(|&index| index != parent.index);
        runtime.current = Some(parent.index);
    }
    parent.thread.lock().set_state(ThreadState::Running);
    let child = Fixture::new(root);
    assert_eq!(RUNTIME.lock().current, Some(parent.index));
    assert_eq!(child.ready_occurrences(), 1);
    RUNTIME.lock().current = None;
    child.dispose();
    parent.dispose();
}

fn admission_capacity(root: &Arc<SpinLock<Job>>) {
    let process = root.lock().create_process("scheduler_capacity").unwrap();
    let mut threads = Vec::new();
    threads.try_reserve_exact(MAX_TASKS).unwrap();
    for _ in 0..MAX_TASKS {
        let thread = process
            .lock()
            .create_named_thread("capacity", 0x20_0040, 4096)
            .unwrap();
        admit(process.clone(), thread.clone()).unwrap();
        threads.push(thread);
    }
    assert_eq!(task_count(), MAX_TASKS);
    let extra = process
        .lock()
        .create_named_thread("excess", 0x20_0040, 4096)
        .unwrap();
    assert_eq!(
        admit(process.clone(), extra.clone()),
        Err(TaskError::QuotaExceeded)
    );
    assert_eq!(task_count(), MAX_TASKS);
    assert_eq!(ready_count(), MAX_TASKS);
    let id = extra.lock().id();
    drop(process.lock().detach_thread(id));
    drop(extra);
    for thread in &threads {
        thread.lock().set_state(ThreadState::Dead);
    }
    reap();
    assert_eq!(task_count(), 0);
    assert_eq!(ready_count(), 0);
    drop(threads);
    drop(process);
}

pub(super) fn run(root: &Arc<SpinLock<Job>>) {
    assert_eq!(task_count(), 0);
    assert!(RUNTIME.lock().current.is_none());
    let processes = root.lock().process_count();
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    #[cfg(feature = "init")]
    init_wait_contract(root);
    rooted_wait_graph(root);
    admission_while_running(root);
    admission_capacity(root);
    startup_stack_pointer_bounds(root);
    failed_task_construction(root);
    notified_close_reuse(root);
    ancestor_revocation(root);
    wait_many_cancellation(root);
    killed_waiter(root);
    sibling_thread_lifetime(root);
    assert_eq!(root.lock().process_count(), processes);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes);
    assert_eq!(task_count(), 0);
    assert_eq!(ready_count(), 0);
    assert!(RUNTIME.lock().current.is_none());
}
