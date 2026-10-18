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
use crate::kernel::sync::SpinLock;
use crate::kernel::task::{Process, ProcessState, TaskError, Thread, ThreadState};

mod tests;

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
    let token = Arc::try_new(WaitToken::new()).map_err(|_| TaskError::OutOfMemory)?;
    let task = Box::try_new(Task {
        process,
        thread,
        token,
        generation: 0,
        pending: None,
    })
    .map_err(|_| TaskError::OutOfMemory)?;
    let mut runtime = RUNTIME.lock();
    assert!(runtime.current.is_none(), "admission during user execution");
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
        runtime
            .tasks
            .try_reserve(1)
            .map_err(|_| TaskError::OutOfMemory)?;
    }
    let capacity = runtime.tasks.len().max(index + 1);
    let additional = capacity.saturating_sub(runtime.ready.len());
    runtime
        .ready
        .try_reserve(additional)
        .map_err(|_| TaskError::OutOfMemory)?;
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
        let (detached, handles, job, process_id, last) = {
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
            (detached, handles, process.job(), process.id(), last)
        };
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

extern "C" fn dispatch_loop() -> ! {
    loop {
        unsafe {
            arch::activate_kernel_address_space();
        }
        RUNTIME.lock().current = None;
        reap();
        reconcile();
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
            unsafe {
                process.lock().address_space().activate();
                arch::bind_user_kernel_stack(top);
            }
            break Some((index, context));
        };
        if let Some((index, context)) = next {
            {
                let mut runtime = RUNTIME.lock();
                runtime.current = Some(index);
                runtime.stats.switches += 1;
            }
            unsafe {
                arch::switch_context(CONTEXTS.dispatcher.get(), context);
            }
        } else {
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
        let (base, size) = thread.user_stack_mapping().expect("user task stack");
        (
            crate::kernel::syscall::USER_PC,
            base + size,
            thread.kernel_stack_top(),
        )
    };
    unsafe { arch::drop_to_user(pc, sp, kernel_sp) }
}

/// Run admitted resident tasks until every task has exited and been reaped.
pub fn run() -> Stats {
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
