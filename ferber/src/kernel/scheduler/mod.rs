//! Single-CPU cooperative execution. Interrupts remain masked while scheduling.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use crate::arch::{self, CpuContext};
use crate::kernel::capabilities::CapNode;
use crate::kernel::ipc::channel::{WaitRegistration, WaitToken};
use crate::kernel::memory::{paging::AddressSpace, Vmo};
use crate::kernel::memory::accounting::{self, AllocError};
use crate::kernel::sync::SpinLock;
use crate::kernel::task::{Process, ProcessState, TaskError, Thread, ThreadState};

mod tests;
mod idle;
mod sources;
#[allow(unused_imports)]
pub(crate) use sources::{register_source, SourceKind, SourceLease};
pub(crate) use idle::MAX_TASKS;

pub fn self_test(root: &Arc<SpinLock<crate::kernel::task::Job>>) {
    tests::run(root);
}

#[derive(Clone)]
struct Pending {
    process_id: u64,
    handle: usize,
    node: Weak<CapNode>,
    registration: WaitRegistration,
    generation: u64,
    error: Option<isize>,
}

struct Task {
    process: Arc<SpinLock<Process>>,
    thread: Arc<SpinLock<Thread>>,
    token: Arc<WaitToken>,
    generation: u64,
    pending: Option<Pending>,
}

#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub switches: usize,
    pub yields: usize,
    pub parks: usize,
    pub wakes: usize,
    pub reaped: usize,
}

struct Runtime {
    tasks: Vec<Option<Box<Task>>>,
    ready: VecDeque<usize>,
    current: Option<usize>,
    stack: Option<Vmo>,
    stats: Stats,
}
static RUNTIME: SpinLock<Runtime> = SpinLock::new(Runtime {
    tasks: Vec::new(),
    ready: VecDeque::new(),
    current: None,
    stack: None,
    stats: Stats {
        switches: 0,
        yields: 0,
        parks: 0,
        wakes: 0,
        reaped: 0,
    },
});

struct Contexts {
    dispatcher: UnsafeCell<CpuContext>,
    launcher: UnsafeCell<CpuContext>,
}
// shortcut: exclusive bootstrap CPU with interrupts masked; replace before IRQ/SMP scheduling.
unsafe impl Sync for Contexts {}
static CONTEXTS: Contexts = Contexts {
    dispatcher: UnsafeCell::new(CpuContext::empty()),
    launcher: UnsafeCell::new(CpuContext::empty()),
};

pub fn init() {
    let owner = accounting::BudgetRef::system();
    accounting::with_budget(&owner, init_in_budget);
}

fn init_in_budget() {
    let mut runtime = RUNTIME.lock();
    if runtime.stack.is_some() {
        return;
    }
    let stack = Vmo::create_contiguous(32768).expect("dispatcher stack");
    let top = crate::kernel::memory::phys_map::phys_to_virt(stack.frames().get(0).unwrap())
        .expect("dispatcher stack alias")
        + stack.size();
    unsafe {
        *CONTEXTS.dispatcher.get() = CpuContext::new(dispatch_loop as *const () as usize, top);
    }
    runtime.stack = Some(stack);
}

/// Admission is fallible; switching and wakeup never allocate queue storage.
pub fn admit(
    process: Arc<SpinLock<Process>>,
    thread: Arc<SpinLock<Thread>>,
) -> Result<(), TaskError> {
    let owner = process.lock().budget().clone();
    let task = accounting::with_budget(&owner, || {
        let token = accounting::attempt(|| Arc::try_new(WaitToken::new())).map_err(alloc_error)?;
        accounting::attempt(|| Box::try_new(Task {
            process, thread, token, generation: 0, pending: None,
        })).map_err(alloc_error)
    })?;
    let mut runtime = RUNTIME.lock();
    if runtime.tasks.iter().flatten().count() >= MAX_TASKS {
        return Err(TaskError::QuotaExceeded);
    }
    assert!(
        !runtime
            .tasks
            .iter()
            .flatten()
            .any(|registered| Arc::ptr_eq(&registered.thread, &task.thread)),
        "duplicate task admission"
    );
    let index = runtime
        .tasks
        .iter()
        .position(Option::is_none)
        .unwrap_or(runtime.tasks.len());
    if index == runtime.tasks.len() {
        accounting::with_budget(&accounting::BudgetRef::system(), ||
            accounting::attempt(|| runtime.tasks.try_reserve(1))).map_err(alloc_error)?;
    }
    let capacity = runtime.tasks.len().max(index + 1);
    let additional = capacity.saturating_sub(runtime.ready.len());
    accounting::with_budget(&accounting::BudgetRef::system(), ||
        accounting::attempt(|| runtime.ready.try_reserve(additional))).map_err(alloc_error)?;
    drop(runtime);
    {
        let mut thread = task.thread.lock();
        assert_eq!(thread.state(), ThreadState::Ready);
        thread.initialize_context(first_entry as *const () as usize);
    }
    let mut runtime = RUNTIME.lock();
    if index == runtime.tasks.len() {
        runtime.tasks.push(Some(task));
    } else {
        runtime.tasks[index] = Some(task);
    }
    runtime.ready.push_back(index);
    Ok(())
}

pub fn ready_count() -> usize {
    RUNTIME.lock().ready.len()
}
pub fn task_count() -> usize {
    RUNTIME.lock().tasks.iter().flatten().count()
}

pub fn current_process() -> Arc<SpinLock<Process>> {
    let runtime = RUNTIME.lock();
    Arc::clone(
        &runtime.tasks[runtime.current.expect("no current task")]
            .as_ref()
            .unwrap()
            .process,
    )
}

#[cfg(feature = "init")]
pub(crate) fn current_identity() -> (u64, u64) {
    let (process, thread) = {
        let runtime = RUNTIME.lock();
        let task = runtime.tasks[runtime.current.expect("no current task")]
            .as_ref()
            .unwrap();
        (Arc::clone(&task.process), Arc::clone(&task.thread))
    };
    let process_id = process.lock().id();
    let thread_id = thread.lock().id();
    (process_id, thread_id)
}

fn current_context() -> *mut CpuContext {
    let thread = {
        let runtime = RUNTIME.lock();
        Arc::clone(
            &runtime.tasks[runtime.current.unwrap()]
                .as_ref()
                .unwrap()
                .thread,
        )
    };
    // The registry pins this allocation, and no guard or Rust reference crosses switch_context.
    let context = unsafe { thread.lock().context_ptr() };
    context
}

fn suspend_state(thread: &Arc<SpinLock<Thread>>, requested: ThreadState) -> ThreadState {
    let mut thread = thread.lock();
    if thread.state() != ThreadState::Dead {
        thread.set_state(requested);
    }
    thread.state()
}

fn suspend(state: ThreadState) {
    {
        let (index, thread) = {
            let runtime = RUNTIME.lock();
            let index = runtime.current.unwrap();
            (
                index,
                Arc::clone(&runtime.tasks[index].as_ref().unwrap().thread),
            )
        };
        let state = suspend_state(&thread, state);
        if state == ThreadState::Ready {
            let mut runtime = RUNTIME.lock();
            assert!(!runtime.ready.contains(&index));
            runtime.ready.push_back(index);
            runtime.stats.yields += 1;
        }
    }
    let context = current_context();
    accounting::assert_switch_safe();
    accounting::set_system();
    unsafe {
        arch::switch_context(context, CONTEXTS.dispatcher.get());
    }
}

pub fn sched_yield() {
    suspend(ThreadState::Ready);
}

pub fn exit() -> ! {
    suspend(ThreadState::Dead);
    panic!("dead thread resumed")
}

pub fn prepare_wait() -> Result<(Arc<WaitToken>, u64), isize> {
    finish_wait();
    let mut runtime = RUNTIME.lock();
    let index = runtime.current.unwrap();
    let task = runtime.tasks[index].as_mut().unwrap();
    let generation = task
        .generation
        .checked_add(1)
        .filter(|g| *g <= u64::MAX >> 1)
        .ok_or(-28isize)?;
    task.generation = generation;
    task.token.start(generation);
    Ok((Arc::clone(&task.token), generation))
}

pub fn park(handle: usize, generation: u64, node: Weak<CapNode>, registration: WaitRegistration) {
    let process_id = {
        let process = current_process();
        let id = process.lock().id();
        id
    };
    {
        let mut runtime = RUNTIME.lock();
        let index = runtime.current.unwrap();
        runtime.tasks[index].as_mut().unwrap().pending = Some(Pending {
            process_id,
            handle,
            node,
            registration,
            generation,
            error: None,
        });
        runtime.stats.parks += 1;
    }
    suspend(ThreadState::Blocked);
}

fn unregister(registration: WaitRegistration, token: Arc<WaitToken>, generation: u64) {
    registration.unregister(&token, generation);
}

pub fn finish_wait() {
    let removed = {
        let mut runtime = RUNTIME.lock();
        let index = runtime.current.expect("wait without caller");
        let task = runtime.tasks[index].as_mut().unwrap();
        task.pending.take().map(|pending| {
            (
                pending.registration,
                Arc::clone(&task.token),
                pending.generation,
            )
        })
    };
    if let Some((node, token, generation)) = removed {
        unregister(node, token, generation);
    }
}

pub fn wait_error() -> Option<isize> {
    let runtime = RUNTIME.lock();
    runtime.tasks[runtime.current.unwrap()]
        .as_ref()
        .unwrap()
        .pending
        .as_ref()
        .and_then(|p| p.error)
}

/// Close cancellation is latched before the slot can be reused, even after a notification.
pub fn cancel_closed(process_id: u64, handle: usize) {
    let count = RUNTIME.lock().tasks.len();
    for index in 0..count {
        let removed = {
            let mut runtime = RUNTIME.lock();
            runtime.tasks[index].as_mut().and_then(|task| {
                let pending = task.pending.as_mut()?;
                if pending.process_id != process_id || pending.handle != handle {
                    return None;
                }
                pending.error = Some(-9);
                task.token.notify(pending.generation);
                Some((
                    pending.registration.clone(),
                    Arc::clone(&task.token),
                    pending.generation,
                ))
            })
        };
        if let Some((node, token, generation)) = removed {
            unregister(node, token, generation);
        }
    }
}

fn reconcile() {
    let count = RUNTIME.lock().tasks.len();
    for index in 0..count {
        let snapshot = {
            let runtime = RUNTIME.lock();
            runtime.tasks[index].as_ref().and_then(|task| {
                Some((
                    Arc::clone(&task.process),
                    Arc::clone(&task.thread),
                    Arc::clone(&task.token),
                    task.pending.as_ref()?.clone(),
                ))
            })
        };
        let Some((process, thread, token, pending)) = snapshot else {
            continue;
        };
        let error = pending.error.or_else(|| {
            let process = process.lock();
            let table = process.handle_table().lock();
            let handle = match u32::try_from(pending.handle) {
                Ok(handle) => handle,
                Err(_) => return Some(-9),
            };
            match table.lookup(handle) {
                Err(crate::kernel::capabilities::CapError::Revoked) => Some(-116),
                Err(_) => Some(-9),
                Ok(node) if !Weak::ptr_eq(&Arc::downgrade(node), &pending.node) => Some(-9),
                Ok(_) => None,
            }
        });
        if let Some(error) = error {
            {
                let mut runtime = RUNTIME.lock();
                runtime.tasks[index]
                    .as_mut()
                    .unwrap()
                    .pending
                    .as_mut()
                    .unwrap()
                    .error = Some(error);
            }
            pending.registration.unregister(&token, pending.generation);
            token.notify(pending.generation);
        }
        if token.is_notified(pending.generation) {
            let mut thread = thread.lock();
            if thread.state() == ThreadState::Blocked {
                thread.set_state(ThreadState::Ready);
                drop(thread);
                let mut runtime = RUNTIME.lock();
                assert!(!runtime.ready.contains(&index));
                runtime.ready.push_back(index);
                runtime.stats.wakes += 1;
            }
        }
    }
}

fn reap() {
    let count = RUNTIME.lock().tasks.len();
    for index in 0..count {
        let dead = {
            let thread = {
                let runtime = RUNTIME.lock();
                runtime.tasks[index].as_ref().map(|t| Arc::clone(&t.thread))
            };
            thread.is_some_and(|t| t.lock().state() == ThreadState::Dead)
        };
        if !dead {
            continue;
        }
        let task = {
            let mut runtime = RUNTIME.lock();
            runtime.ready.retain(|&id| id != index);
            runtime.stats.reaped += 1;
            runtime.tasks[index].take().unwrap()
        };
        if let Some(pending) = &task.pending {
            unregister(
                pending.registration.clone(),
                Arc::clone(&task.token),
                pending.generation,
            );
        }
        let (id, mapping) = {
            let thread = task.thread.lock();
            (thread.id(), thread.user_stack_mapping())
        };
        let (detached, handles, mappings, job, process_id, last) = {
            let mut process = task.process.lock();
            if let Some((base, size)) = mapping {
                unsafe {
                    process
                        .address_space_mut()
                        .unmap_vmo(base, size.div_ceil(4096))
                        .expect("unmap retired stack");
                }
            }
            let detached = process.detach_thread(id);
            let last = process.thread_count() == 0;
            let handles = if last {
                process.set_state(ProcessState::Dead);
                Some(process.take_handles())
            } else {
                None
            };
            let mappings = if last { process.take_mappings() } else { Vec::new() };
            (detached, handles, mappings, process.job(), process.id(), last)
        };
        drop(mappings);
        drop(handles);
        drop(detached);
        if last {
            if let Some(job) = job.and_then(|job| job.upgrade()) {
                let removed = job.lock().detach_process(process_id);
                drop(removed);
            }
        }
        drop(task);
    }
}

/// Every blocked component must reach a concrete kernel producer, not merely a live peer.
pub(crate) fn idle_quiescent() -> bool {
    use crate::kernel::capabilities::Rights;
    type Snapshot = (Arc<SpinLock<Process>>, Arc<SpinLock<Thread>>, Arc<WaitToken>, Option<Pending>, u64);
    let mut tasks: [Option<Snapshot>; MAX_TASKS] = core::array::from_fn(|_| None);
    let count = {
        let runtime = RUNTIME.lock();
        if runtime.current.is_some() || !runtime.ready.is_empty() { return false; }
        let mut count = 0;
        for task in runtime.tasks.iter().flatten() {
            if count == MAX_TASKS { return false; }
            tasks[count] = Some((Arc::clone(&task.process), Arc::clone(&task.thread),
                Arc::clone(&task.token), task.pending.clone(), task.generation));
            count += 1;
        }
        count
    };
    let sources = sources::snapshot();
    let mut edges = [0u64; MAX_TASKS];
    let mut roots = 0u64;
    for (index, task) in tasks.iter().take(count).enumerate() {
        let (process, thread, token, pending, generation) = task.as_ref().unwrap();
        let Some(pending) = pending else { return false; };
        if pending.generation != *generation || pending.error.is_some()
            || token.is_notified(*generation) || thread.lock().state() != ThreadState::Blocked
            || !pending.registration.registered(token, *generation)
        { return false; }
        {
            let process = process.lock();
            if process.state() != ProcessState::Running || process.id() != pending.process_id { return false; }
            let table = process.handle_table().lock();
            let Ok(handle) = u32::try_from(pending.handle) else { return false; };
            let Ok(node) = table.lookup(handle) else { return false; };
            if !Arc::downgrade(node).ptr_eq(&pending.node)
                || !node.rights().contains(Rights::READ | Rights::WAIT)
                || !node.object_ref::<super::ipc::channel::Channel>().is_some_and(|endpoint|
                    pending.registration.same_endpoint(&endpoint.wait_registration()))
            { return false; }
        }
        if sources.iter().flatten().any(|source|
            source.live() && pending.registration.peer_of(source))
        { roots |= 1u64 << index; }
        for (producer, task) in tasks.iter().take(count).enumerate() {
            let candidate = task.as_ref().unwrap().0.lock();
            if candidate.state() == ProcessState::Dead { continue; }
            let table = candidate.handle_table().lock();
            if table.nodes().any(|node| node.is_valid() && node.rights().contains(Rights::WRITE)
                && node.object_ref::<super::ipc::channel::Channel>().is_some_and(|endpoint|
                    pending.registration.peer_of(&endpoint.wait_registration())))
            { edges[index] |= 1u64 << producer; }
        }
    }
    idle::rooted(count, &edges, roots)
}

#[cfg(feature = "init")]
fn init_wait_matches(session: &super::init::Session) -> bool {
    use super::capabilities::Rights;
    let Some(peer) = &session.peer else {
        return false;
    };
    let snapshot = {
        let runtime = RUNTIME.lock();
        if runtime.current.is_some() || !runtime.ready.is_empty() {
            return false;
        }
        let Some(task) = runtime.tasks.iter().flatten().find(|task|
            task.pending.as_ref().is_some_and(|pending| pending.process_id == session.admitted.process_id))
        else { return false; };
        let Some(pending) = task.pending.clone() else {
            return false;
        };
        if pending.generation != task.generation
            || pending.error.is_some()
            || task.token.is_notified(pending.generation)
            || !pending.registration.waits_on_peer_of(peer)
            || !pending.node.ptr_eq(&session.node)
        {
            return false;
        }
        (
            Arc::clone(&task.process),
            Arc::clone(&task.thread),
            Arc::clone(&task.token),
            pending,
        )
    };
    let (process, thread, token, pending) = snapshot;
    if !pending
        .registration
        .is_registered_peer_wait(peer, &token, pending.generation)
    {
        return false;
    }
    let thread = thread.lock();
    if thread.id() != session.admitted.thread_id || thread.state() != ThreadState::Blocked {
        return false;
    }
    drop(thread);
    let process = process.lock();
    if process.id() != session.admitted.process_id
        || pending.process_id != process.id()
        || process.state() != ProcessState::Running
    {
        return false;
    }
    let table = process.handle_table().lock();
    let Ok(handle) = u32::try_from(pending.handle) else {
        return false;
    };
    let Ok(node) = table.lookup(handle) else {
        return false;
    };
    Arc::downgrade(node).ptr_eq(&session.node)
        && node
            .rights()
            .contains(Rights::READ | Rights::WRITE | Rights::WAIT)
        && node
            .object_ref::<super::ipc::channel::Channel>()
            .is_some_and(|endpoint| endpoint.wait_registration().waits_on_peer_of(peer))
}

#[cfg(feature = "init")]
fn service_wait_matches(session: &super::init::Session) -> bool {
    use super::capabilities::Rights;
    let Some(process_id) = session.service_process_id() else {
        return false;
    };
    let Some(endpoint) = session.service_channel() else {
        return false;
    };
    let endpoint_registration = endpoint.wait_registration();
    let (process, thread, token, pending, generation) = {
        let runtime = RUNTIME.lock();
        if runtime.current.is_some() || !runtime.ready.is_empty() {
            return false;
        }
        let Some(task) = runtime.tasks.iter().flatten().find(|task| {
            task.pending.as_ref().is_some_and(|pending| pending.process_id == process_id)
        }) else {
            return false;
        };
        let Some(pending) = task.pending.clone() else {
            return false;
        };
        (
            Arc::clone(&task.process),
            Arc::clone(&task.thread),
            Arc::clone(&task.token),
            pending,
            task.generation,
        )
    };
    if pending.generation != generation
        || pending.error.is_some()
        || token.is_notified(pending.generation)
        || !pending.registration.same_endpoint(&endpoint_registration)
        || !pending.registration.registered(&token, generation)
        || thread.lock().state() != ThreadState::Blocked
    {
        return false;
    }
    let process = process.lock();
    if process.id() != process_id || process.state() != ProcessState::Running {
        return false;
    }
    let Ok(handle) = u32::try_from(pending.handle) else {
        return false;
    };
    let table = process.handle_table().lock();
    let Ok(node) = table.lookup(handle) else {
        return false;
    };
    Arc::downgrade(node).ptr_eq(&pending.node)
        && node.rights().contains(Rights::READ | Rights::WAIT)
        && node
            .object_arc::<super::ipc::channel::Channel>()
            .is_some_and(|object| Arc::ptr_eq(&object, &endpoint))
}

#[cfg(feature = "init")]
fn finish_init(mut session: super::init::Session) -> bool {
    if session.closure_probe
        && session.parked_once
        && task_count() == 0
        && session.exit_status == Some(1)
        && session.service_exit_status() == Some(0)
    {
        return true;
    }
    if session.closure_probe && session.exit_status == Some(1) {
        if let Some(status) = session.service_exit_status().filter(|status| *status != 0) {
            panic!("initial service exited unexpectedly: {status}");
        }
    }
    let ready = session.exit_status.is_none() && session.ready();
    if ready && init_wait_matches(&session) && service_wait_matches(&session) && idle_quiescent() {
        if session.closure_probe {
            session.parked_once = true;
            assert!(session.begin_shutdown(), "init shutdown delivery");
            super::init::install(session);
            return false;
        }
        if session.before_quiescent() {
            super::init::install(session);
            return false;
        }
        #[cfg(feature = "service-wake-probe")]
        assert!(session.software_woke(), "init software event did not complete");
        crate::kprintln!("[FERBER_TEST] boot_payload: PASS");
        #[cfg(feature = "service-wake-probe")]
        crate::kprintln!("[FERBER_TEST] software_wake: PASS");
        crate::kprintln!("[FERBER_TEST] service_launch: PASS");
        crate::kprintln!("[FERBER_TEST] multiservice_idle: PASS");
        crate::kprintln!("[FERBER_TEST] init_ready: PASS");
        crate::kprintln!("[FERBER_TEST] init_waiting: PASS");
        // shortcut: no IRQ/event producer exists yet; replace terminal halt before event-driven init.
        super::syscall::halt();
    }
    // Dispatcher stack and kernel root are active; no retired stack is freed underneath us.
    let count = RUNTIME.lock().tasks.len();
    for index in 0..count {
        let thread = RUNTIME.lock().tasks[index]
            .as_ref()
            .map(|task| Arc::clone(&task.thread));
        if let Some(thread) = thread {
            thread.lock().set_state(ThreadState::Dead);
        }
    }
    reap();
    let status = session.exit_status;
    let service_status = session.service_exit_status();
    let announced = session.announced_ready();
    drop(session);
    if let Some(status) = status {
        panic!("init exited unexpectedly: {status}");
    }
    if let Some(status) = service_status {
        panic!("initial service exited unexpectedly: {status}");
    }
    if announced {
        panic!("cooperative deadlock: live tasks with no runnable producer");
    }
    panic!("init bootstrap failed");
}

extern "C" fn dispatch_loop() -> ! {
    loop {
        accounting::assert_switch_safe();
        accounting::set_system();
        unsafe {
            arch::activate_kernel_address_space();
        }
        RUNTIME.lock().current = None;
        reap();
        reconcile();
        #[cfg(feature = "init")]
        if super::init::pump() { reconcile(); }
        let next = loop {
            let index = RUNTIME.lock().ready.pop_front();
            let Some(index) = index else {
                break None;
            };
            let candidate = {
                let runtime = RUNTIME.lock();
                runtime.tasks[index]
                    .as_ref()
                    .map(|t| (Arc::clone(&t.process), Arc::clone(&t.thread)))
            };
            let Some((process, thread)) = candidate else {
                continue;
            };
            assert_ne!(process.lock().state(), ProcessState::Dead);
            let mut state = thread.lock();
            if state.state() != ThreadState::Ready {
                continue;
            }
            state.set_state(ThreadState::Running);
            let context = unsafe { state.context_ptr() };
            let top = state.kernel_stack_top();
            drop(state);
            let owner = process.lock().budget().clone();
            unsafe {
                process.lock().address_space().activate();
                arch::bind_user_kernel_stack(top);
            }
            break Some((index, context, owner));
        };
        if let Some((index, context, owner)) = next {
            {
                let mut runtime = RUNTIME.lock();
                runtime.current = Some(index);
                runtime.stats.switches += 1;
            }
            accounting::assert_switch_safe();
            accounting::set_current(&owner);
            drop(owner);
            unsafe {
                arch::switch_context(CONTEXTS.dispatcher.get(), context);
            }
        } else {
            #[cfg(feature = "init")]
            if let Some(session) = super::init::take() {
                if finish_init(session) {
                    unsafe {
                        arch::switch_context(CONTEXTS.dispatcher.get(), CONTEXTS.launcher.get());
                    }
                }
                continue;
            }
            if task_count() != 0 {
                panic!("cooperative deadlock: live tasks with no runnable producer");
            }
            unsafe {
                arch::switch_context(CONTEXTS.dispatcher.get(), CONTEXTS.launcher.get());
            }
        }
    }
}

extern "C" fn first_entry() -> ! {
    let (pc, sp, kernel_sp) = {
        let (process, thread) = {
            let runtime = RUNTIME.lock();
            let task = runtime.tasks[runtime.current.unwrap()].as_ref().unwrap();
            (Arc::clone(&task.process), Arc::clone(&task.thread))
        };
        process.lock().set_state(ProcessState::Running);
        let thread = thread.lock();
        (
            thread.user_entry(),
            thread.user_initial_sp().expect("user task initial stack"),
            thread.kernel_stack_top(),
        )
    };
    unsafe { arch::drop_to_user(pc, sp, kernel_sp) }
}

/// Run admitted tasks until every task has exited and been reaped.
pub fn run() -> Stats {
    accounting::assert_switch_safe();
    accounting::set_system();
    {
        let mut runtime = RUNTIME.lock();
        assert!(runtime.current.is_none());
        runtime.stats = Stats::default();
    }
    unsafe {
        arch::switch_context(CONTEXTS.launcher.get(), CONTEXTS.dispatcher.get());
    }
    assert_eq!(task_count(), 0);
    RUNTIME.lock().stats
}

#[cfg(feature = "init")]
pub(crate) fn run_init(session: super::init::Session) -> ! {
    super::init::install(session);
    run();
    panic!("init scheduler returned")
}

#[cfg(feature = "init")]
pub(crate) fn run_init_closure_probe(session: super::init::Session) {
    assert!(session.closure_probe);
    super::init::install(session);
    let stats = run();
    assert_eq!(stats.reaped, 2, "init closure did not reap init and service");
}

fn alloc_error(error: AllocError) -> TaskError {
    match error { AllocError::Quota => TaskError::QuotaExceeded, AllocError::OutOfMemory => TaskError::OutOfMemory }
}
