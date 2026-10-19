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
                Rights::READ | Rights::WAIT | Rights::DUPLICATE | Rights::MANAGE,
            )
            .unwrap();
        let handle = process
            .lock()
            .handle_table()
            .lock()
            .insert(node.clone())
            .unwrap();
        admit(process.clone(), thread.clone()).unwrap();
        assert_eq!(thread.lock().user_entry(), ENTRY, "admission lost user entry");
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
        assert_eq!(process.reserve_backing(usize::MAX), Err(TaskError::OutOfMemory));
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
    assert!(weak.upgrade().is_none(), "failed construction retained process");
}

pub(super) fn run(root: &Arc<SpinLock<Job>>) {
    assert_eq!(task_count(), 0);
    assert!(RUNTIME.lock().current.is_none());
    let processes = root.lock().process_count();
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    failed_task_construction(root);
    notified_close_reuse(root);
    ancestor_revocation(root);
    killed_waiter(root);
    sibling_thread_lifetime(root);
    assert_eq!(root.lock().process_count(), processes);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes);
    assert_eq!(task_count(), 0);
    assert_eq!(ready_count(), 0);
    assert!(RUNTIME.lock().current.is_none());
}
