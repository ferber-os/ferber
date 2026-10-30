//! Staged task construction. Publication is the final, fallible operation.
use super::capabilities::{
    CapNode, KernelObject, ObjectType, Resource, Rights, CAP_NODE_ALLOCATOR,
};
use super::ipc::channel::Channel;
use super::memory::vmo::{VmoError, MAX_USER_VMO_SIZE};
use super::memory::{
    accounting,
    paging::{AddressSpace, MapFlags, PageError},
    user_mem, Vmo,
};
use super::scheduler;
use super::sync::SpinLock;
use super::syscall::{allocation_errno, cap_errno, STACK_SIZE, USER_STACK};
use super::task::process::MappingRecord;
use super::task::{Job, JobState, Process, ProcessState, TaskError};
use alloc::sync::{Arc, Weak};

use super::startup;

pub struct JobAuthority {
    job: Weak<SpinLock<Job>>,
}
impl JobAuthority {
    #[cfg_attr(not(feature = "init"), allow(dead_code))]
    pub fn new(job: &Arc<SpinLock<Job>>) -> Self {
        Self {
            job: Arc::downgrade(job),
        }
    }
}
impl KernelObject for JobAuthority {
    fn type_name(&self) -> &'static str {
        "Job"
    }
}

#[cfg_attr(not(feature = "init"), allow(dead_code))]
pub fn job_capability(job: &Arc<SpinLock<Job>>) -> Result<Arc<CapNode>, isize> {
    let object =
        accounting::attempt(|| Arc::try_new(JobAuthority::new(job))).map_err(allocation_errno)?;
    CAP_NODE_ALLOCATOR
        .alloc_root(
            object,
            ObjectType::Job,
            Rights::MANAGE | Rights::DUPLICATE | Rights::INSPECT,
        )
        .map_err(cap_errno)
}

struct Draft {
    process: Arc<SpinLock<Process>>,
    bootstrap: Option<u32>,
}
enum Phase {
    Draft(Draft),
    Started { _process: Weak<SpinLock<Process>> },
    Aborted,
}
pub struct ProcessController {
    authority: Arc<CapNode>,
    state: SpinLock<Phase>,
}
impl KernelObject for ProcessController {
    fn type_name(&self) -> &'static str {
        "Process"
    }
}

fn retire(process: Arc<SpinLock<Process>>) {
    let (job, id) = {
        let mut process = process.lock();
        process.kill();
        (process.job(), process.id())
    };
    if let Some(job) = job.and_then(|job| job.upgrade()) {
        let removed = job.lock().detach_process(id);
        drop(removed);
    }
    // The inactive address space drops before its mapped VMO and stack owners.
    drop(process);
}
impl Drop for ProcessController {
    fn drop(&mut self) {
        let state = core::mem::replace(&mut *self.state.lock(), Phase::Aborted);
        if let Phase::Draft(draft) = state {
            retire(draft.process);
        }
    }
}
impl ProcessController {
    fn authorize(&self) -> Result<(), isize> {
        if !self.authority.is_valid() {
            return Err(-116);
        }
        let authority = self
            .authority
            .object_ref::<JobAuthority>()
            .ok_or(-22isize)?;
        let job = authority.job.upgrade().ok_or(-16isize)?;
        let job = job.lock();
        if job.state() != JobState::Active || job.budget().is_closed() {
            return Err(-16);
        }
        Ok(())
    }
}

fn node(
    process: &Arc<SpinLock<Process>>,
    raw: usize,
    rights: Rights,
    kind: ObjectType,
) -> Result<Arc<CapNode>, isize> {
    let handle = u32::try_from(raw).map_err(|_| -9isize)?;
    let process = process.lock();
    let table = process.handle_table().lock();
    let node = table.lookup(handle).map_err(cap_errno)?;
    if !node.rights().contains(rights) {
        return Err(-13);
    }
    if node.object_type() != kind {
        return Err(-22);
    }
    Ok(Arc::clone(node))
}
fn controller(
    process: &Arc<SpinLock<Process>>,
    raw: usize,
    rights: Rights,
) -> Result<Arc<ProcessController>, isize> {
    node(process, raw, rights, ObjectType::Process)?
        .object_arc::<ProcessController>()
        .ok_or(-22)
}
fn task_errno(error: TaskError) -> isize {
    match error {
        TaskError::OutOfMemory => -12,
        TaskError::QuotaExceeded => -28,
        _ => -16,
    }
}
fn page_errno(error: PageError) -> isize {
    match error {
        PageError::OutOfMemory => -12,
        PageError::JobQuota | PageError::TableBudget => -28,
        PageError::AlreadyMapped => -17,
        PageError::Unaligned => -22,
        _ => -14,
    }
}
fn vmo_errno(error: VmoError) -> isize {
    match error {
        VmoError::OutOfMemory => -12,
        VmoError::JobQuota | VmoError::UserBudget => -28,
        VmoError::PermissionDenied => -13,
        VmoError::TooLarge => -90,
        _ => -22,
    }
}
fn draft(state: &mut Phase) -> Result<&mut Draft, isize> {
    match state {
        Phase::Draft(draft) if draft.process.lock().state() != ProcessState::Dead => Ok(draft),
        _ => Err(-16),
    }
}

pub fn dispatch(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    let used = match nr {
        17 | 21 | 22 => 2,
        18 | 19 => 3,
        20 => 4,
        _ => return -38,
    };
    if args[used..].iter().any(|arg| *arg != 0) {
        return -22;
    }
    let result = match nr {
        17 => create(process, args[0], args[1]),
        18 => map(process, args[0], args[1], args[2]).map(|()| 0),
        19 => {
            if args[2] == 0 {
                give(process, args[0], args[1])
            } else {
                Err(-22)
            }
        }
        20 => prepare(process, args[0], args[1], args[2], args[3]).map(|()| 0),
        21 => start(process, args[0], args[1]).map(|()| 0),
        22 => abort(process, args[0], args[1]).map(|()| 0),
        _ => unreachable!(),
    };
    result.unwrap_or_else(|error| error)
}
fn create(caller: &Arc<SpinLock<Process>>, raw: usize, options: usize) -> Result<isize, isize> {
    if options != 0 {
        return Err(-22);
    }
    let authority = node(caller, raw, Rights::MANAGE, ObjectType::Job)?;
    let job = authority
        .object_ref::<JobAuthority>()
        .ok_or(-22isize)?
        .job
        .upgrade()
        .ok_or(-16isize)?;
    if caller.lock().handle_table().lock().is_full() {
        return Err(-24);
    }
    let process = job.lock().create_process("service").map_err(task_errno)?;
    let object = accounting::attempt(|| {
        Arc::try_new(ProcessController {
            authority,
            state: SpinLock::new(Phase::Draft(Draft {
                process,
                bootstrap: None,
            })),
        })
    })
    .map_err(allocation_errno)?;
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(
            object,
            ObjectType::Process,
            Rights::MANAGE | Rights::MAP | Rights::EXECUTE | Rights::DUPLICATE | Rights::INSPECT,
        )
        .map_err(cap_errno)?;
    let result = {
        let caller = caller.lock();
        let mut table = caller.handle_table().lock();
        table.insert(node)
    };
    match result {
        Ok(slot) => Ok(slot as isize),
        Err((error, node)) => {
            drop(node);
            Err(cap_errno(error))
        }
    }
}
fn map(
    caller: &Arc<SpinLock<Process>>,
    raw: usize,
    vmo: usize,
    address: usize,
) -> Result<(), isize> {
    let controller = controller(caller, raw, Rights::MANAGE | Rights::MAP)?;
    controller.authorize()?;
    let source = node(caller, vmo, Rights::MAP, ObjectType::Vmo)?;
    let object = source.object_arc::<Vmo>().ok_or(-22isize)?;
    if !object.is_owned_ram() {
        return Err(-22);
    }
    let mut bytes = [0; 48];
    {
        let caller = caller.lock();
        unsafe {
            user_mem::copy_from_user(caller.address_space(), &mut bytes, address)
                .map_err(|_| -14isize)?;
        }
    }
    let mut fields = [0usize; 6];
    for (field, bytes) in fields.iter_mut().zip(bytes.as_chunks::<8>().0) {
        *field = u64::from_le_bytes(*bytes) as usize;
    }
    let [base, offset, len, permissions, options, reserved] = fields;
    if options != 0
        || reserved != 0
        || !matches!(permissions, 1 | 3 | 5)
        || len == 0
        || (base | offset | len) & 4095 != 0
    {
        return Err(-22);
    }
    if len > MAX_USER_VMO_SIZE {
        return Err(-90);
    }
    let flags = match permissions {
        3 => MapFlags::user_rw(),
        5 => MapFlags::user_rx(),
        _ => MapFlags::user_ro(),
    };
    Vmo::check_mapping_rights(source.rights(), flags).map_err(|_| -13isize)?;
    object
        .validate_owned_range(offset, len)
        .map_err(vmo_errno)?;
    let ceiling = if cfg!(target_arch = "riscv64") {
        1usize << 38
    } else {
        1usize << 47
    };
    let end = base
        .checked_add(len)
        .filter(|end| base >= 4096 && *end <= ceiling)
        .ok_or(-14isize)?;
    if base < USER_STACK + STACK_SIZE && USER_STACK - 4096 < end {
        return Err(-22);
    }
    let mut state = controller.state.lock();
    let draft = draft(&mut state)?;
    let mut child = draft.process.lock();
    let owner = child.budget().clone();
    accounting::with_budget(&owner, || {
        child
            .address_space()
            .check_free_range(base, len / 4096)
            .map_err(page_errno)?;
        if child.mappings().len() >= 64
            || child.mappings().iter().map(|r| r.len / 4096).sum::<usize>() + len / 4096 > 16384
        {
            return Err(-28);
        }
        let permit = object.mapping_permit(flags).map_err(vmo_errno)?;
        accounting::attempt(|| child.mappings_mut().try_reserve(1)).map_err(allocation_errno)?;
        unsafe {
            child
                .address_space_mut()
                .map_pages(base, object.frames(), offset / 4096, len / 4096, flags)
                .map_err(page_errno)?;
        }
        child.mappings_mut().push(MappingRecord {
            base,
            offset,
            len,
            flags,
            object: Arc::clone(&object),
            _writable: permit,
        });
        Ok(())
    })
}
fn give(caller: &Arc<SpinLock<Process>>, raw: usize, handle: usize) -> Result<isize, isize> {
    let controller = controller(caller, raw, Rights::MANAGE)?;
    controller.authorize()?;
    let handle = u32::try_from(handle).map_err(|_| -9isize)?;
    let mut state = controller.state.lock();
    let draft = draft(&mut state)?;
    let caller = caller.lock();
    let mut source = caller.handle_table().lock();
    let node = source.lookup(handle).map_err(cap_errno)?;
    if !node.rights().contains(Rights::TRANSFER) {
        return Err(-13);
    }
    let valid = match node.object_type() {
        ObjectType::Vmo => node.object_ref::<Vmo>().is_some_and(Vmo::is_owned_ram),
        ObjectType::Resource => node.object_ref::<Resource>().is_some(),
        ObjectType::Channel => node.object_ref::<Channel>().is_some(),
        _ => false,
    };
    if !valid {
        return Err(-22);
    }
    let child = draft.process.lock();
    let mut target = child.handle_table().lock();
    let mut slots = [0];
    target.free_slots(1, &mut slots).map_err(cap_errno)?;
    let node = source.remove(handle).map_err(cap_errno)?;
    target.insert_at(slots[0], node);
    Ok(slots[0] as isize)
}
fn entry_valid(process: &Process, entry: usize) -> Result<(), isize> {
    if entry == 0 || (cfg!(target_arch = "riscv64") && entry & 1 != 0) {
        return Err(-22);
    }
    let (_, flags) = process.address_space().query(entry).ok_or(-14isize)?;
    if !flags.user || !flags.readable || !flags.executable || flags.writable {
        return Err(-13);
    }
    if !process.mappings().iter().any(|record| {
        record.base <= entry
            && entry < record.base + record.len
            && record.flags.executable
            && record.object.is_sealed()
    }) {
        return Err(-13);
    }
    Ok(())
}
fn bootstrap_valid(process: &Process, slot: Option<u32>) -> Result<(), isize> {
    if let Some(slot) = slot {
        let table = process.handle_table().lock();
        let node = table.lookup(slot).map_err(cap_errno)?;
        if node.object_type() != ObjectType::Channel || node.object_ref::<Channel>().is_none() {
            return Err(-22);
        }
        if !node
            .rights()
            .contains(Rights::READ | Rights::WRITE | Rights::WAIT)
        {
            return Err(-13);
        }
    }
    Ok(())
}
fn prepare(
    caller: &Arc<SpinLock<Process>>,
    raw: usize,
    entry: usize,
    bootstrap: usize,
    options: usize,
) -> Result<(), isize> {
    if options != 0 {
        return Err(-22);
    }
    let controller = controller(caller, raw, Rights::MANAGE)?;
    controller.authorize()?;
    let mut state = controller.state.lock();
    let draft = draft(&mut state)?;
    let mut process = draft.process.lock();
    if process.thread_count() != 0 {
        return Err(-16);
    }
    let bootstrap = if bootstrap == usize::MAX {
        None
    } else {
        Some(u32::try_from(bootstrap).map_err(|_| -9isize)?)
    };
    entry_valid(&process, entry)?;
    bootstrap_valid(&process, bootstrap)?;
    process
        .address_space()
        .check_free_range(USER_STACK - 4096, STACK_SIZE / 4096 + 1)
        .map_err(page_errno)?;
    let owner = process.budget().clone();
    accounting::with_budget(&owner, || {
        let mut stack = Vmo::create(STACK_SIZE).map_err(vmo_errno)?;
        stack
            .write(
                STACK_SIZE - startup::SIZE,
                &startup::Startup { bootstrap }.encode(),
            )
            .map_err(vmo_errno)?;
        unsafe {
            process
                .address_space_mut()
                .map_vmo(USER_STACK, stack.frames(), MapFlags::user_rw())
                .map_err(page_errno)?;
        }
        let thread = match process.create_thread(entry, STACK_SIZE) {
            Ok(thread) => thread,
            Err(error) => {
                unsafe {
                    process
                        .address_space_mut()
                        .unmap_vmo(USER_STACK, STACK_SIZE / 4096)
                        .expect("unmap failed initial stack");
                }
                return Err(task_errno(error));
            }
        };
        let mut thread = thread.lock();
        thread.set_user_stack(stack);
        thread.set_user_stack_mapping(USER_STACK);
        thread
            .set_user_initial_sp(USER_STACK + STACK_SIZE - startup::SIZE)
            .expect("fixed initial stack");
        draft.bootstrap = bootstrap;
        Ok(())
    })
}
fn start(caller: &Arc<SpinLock<Process>>, raw: usize, options: usize) -> Result<(), isize> {
    if options != 0 {
        return Err(-22);
    }
    let controller = controller(caller, raw, Rights::MANAGE | Rights::EXECUTE)?;
    controller.authorize()?;
    let mut state = controller.state.lock();
    let draft = draft(&mut state)?;
    let thread = {
        let process = draft.process.lock();
        if process.thread_count() != 1 {
            return Err(-16);
        }
        let thread = Arc::clone(&process.threads()[0]);
        entry_valid(&process, thread.lock().user_entry())?;
        bootstrap_valid(&process, draft.bootstrap)?;
        thread
    };
    scheduler::admit(Arc::clone(&draft.process), thread).map_err(task_errno)?;
    let weak = Arc::downgrade(&draft.process);
    *state = Phase::Started { _process: weak };
    Ok(())
}
fn abort(caller: &Arc<SpinLock<Process>>, raw: usize, options: usize) -> Result<(), isize> {
    if options != 0 {
        return Err(-22);
    }
    let controller = controller(caller, raw, Rights::MANAGE)?;
    let retired = {
        let mut state = controller.state.lock();
        match &*state {
            Phase::Started { .. } => return Err(-16),
            Phase::Aborted => return Ok(()),
            Phase::Draft(_) => {}
        }
        match core::mem::replace(&mut *state, Phase::Aborted) {
            Phase::Draft(draft) => draft.process,
            _ => unreachable!(),
        }
    };
    retire(retired);
    Ok(())
}
