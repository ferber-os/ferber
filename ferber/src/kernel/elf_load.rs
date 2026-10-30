//! Kernel-constructed static ELF tasks. Nothing is runnable before admission.

use alloc::sync::Arc;

pub(crate) use super::startup;

use super::capabilities::CapNode;
use super::elf::{ElfError, ElfImage};
use super::memory::{
    paging::{AddressSpace, MapFlags, PageError},
    Vmo, VmoError,
};
use super::scheduler;
use super::sync::SpinLock;
use super::syscall::{STACK_SIZE, USER_STACK};
use super::task::{Job, Process, TaskError, Thread};

#[cfg(target_arch = "riscv64")]
const MACHINE: u16 = 243;
#[cfg(target_arch = "x86_64")]
const MACHINE: u16 = 62;
#[cfg(target_arch = "riscv64")]
const USER_END: usize = 1 << 38;
#[cfg(target_arch = "x86_64")]
const USER_END: usize = 1 << 47;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Image(ElfError),
    StackOverlap,
    Memory(VmoError),
    Mapping(PageError),
    Task(TaskError),
}

#[cfg_attr(any(feature = "init", feature = "userspace-job-accounting-probe"), allow(dead_code))]
pub fn admit(
    root: &Arc<SpinLock<Job>>,
    bytes: &[u8],
    bootstrap: Option<Arc<CapNode>>,
) -> Result<(), LoadError> {
    admit_named(root, bytes, bootstrap, "elf_probe").map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub process_id: u64,
    pub thread_id: u64,
}

pub fn admit_named(
    root: &Arc<SpinLock<Job>>,
    bytes: &[u8],
    bootstrap: Option<Arc<CapNode>>,
    name: &'static str,
) -> Result<Admitted, LoadError> {
    let image = ElfImage::parse(bytes, MACHINE, USER_END).map_err(LoadError::Image)?;
    // Keep an unmapped guard page below the fixed test stack.
    for segment in image.segments() {
        if segment.page_start < USER_STACK + STACK_SIZE
            && USER_STACK - 4096 < segment.page_start + segment.page_size
        {
            return Err(LoadError::StackOverlap);
        }
    }
    let process = root.lock().create_process(name).map_err(LoadError::Task)?;
    let id = process.lock().id();
    let result = populate(&process, &image, bootstrap, name).and_then(|thread| {
        let thread_id = thread.lock().id();
        scheduler::admit(Arc::clone(&process), thread).map_err(LoadError::Task)?;
        Ok(Admitted {
            process_id: id,
            thread_id,
        })
    });
    if result.is_err() {
        process.lock().kill();
        // Job owns a strong reference from creation, even before admission.
        let detached = root.lock().detach_process(id);
        drop(detached);
    }
    result
}

fn populate(
    process: &Arc<SpinLock<Process>>,
    image: &ElfImage<'_>,
    bootstrap: Option<Arc<CapNode>>,
    name: &'static str,
) -> Result<Arc<SpinLock<Thread>>, LoadError> {
    let owner = process.lock().budget().clone();
    super::memory::accounting::with_budget(&owner, || populate_in_budget(process, image, bootstrap, name))
}

fn populate_in_budget(
    process: &Arc<SpinLock<Process>>,
    image: &ElfImage<'_>,
    bootstrap: Option<Arc<CapNode>>,
    name: &'static str,
) -> Result<Arc<SpinLock<Thread>>, LoadError> {
    let mut process = process.lock();
    process
        .reserve_backing(image.segments().count())
        .map_err(LoadError::Task)?;
    for segment in image.segments() {
        let mut backing = Vmo::create(segment.page_size).map_err(LoadError::Memory)?;
        let offset = segment.virtual_address - segment.page_start;
        debug_assert!(offset + segment.memory_size <= backing.size());
        backing
            .write(offset, segment.data)
            .map_err(LoadError::Memory)?;
        let flags = MapFlags {
            readable: segment.readable,
            writable: segment.writable,
            executable: segment.executable,
            user: true,
        };
        unsafe {
            process
                .address_space_mut()
                .map_vmo(segment.page_start, backing.frames(), flags)
                .map_err(LoadError::Mapping)?;
        }
        // The ownership slot was reserved before any mapping was installed.
        process.retain_backing(backing);
    }
    let thread = process
        .create_named_thread(name, image.entry(), STACK_SIZE)
        .map_err(LoadError::Task)?;
    assert_eq!(process.handle_table().lock().count(), 0, "ELF birth table");
    let handle = bootstrap.map(|node| {
        // Exercise discovery through startup metadata rather than a fixed ABI slot.
        let slot = 7;
        process.handle_table().lock().insert_at(slot, node);
        slot
    });
    let mut stack = Vmo::create(STACK_SIZE).map_err(LoadError::Memory)?;
    stack
        .write(
            STACK_SIZE - startup::SIZE,
            &startup::Startup { bootstrap: handle }.encode(),
        )
        .map_err(LoadError::Memory)?;
    unsafe {
        process
            .address_space_mut()
            .map_vmo(USER_STACK, stack.frames(), MapFlags::user_rw())
            .map_err(LoadError::Mapping)?;
    }
    {
        let mut thread = thread.lock();
        thread.set_user_stack(stack);
        thread.set_user_stack_mapping(USER_STACK);
        thread
            .set_user_initial_sp(USER_STACK + STACK_SIZE - startup::SIZE)
            .expect("validated ELF initial stack");
    }
    Ok(thread)
}
