//! Ferber syscall ABI. Object operations require explicit handles; yield and
//! exit operate on the current task. Linux-style registers, Ferber numbers.

use crate::arch::{self, ArchAddressSpace};
use crate::kernel::capabilities::{CapError, Rights};
use crate::kernel::memory::user_mem::copy_to_user;
use crate::kernel::memory::{
    bitmap,
    paging::{AddressSpace, MapFlags},
    Vmo,
};
use crate::kernel::sync::SpinLock;
use crate::kernel::task::{Job, Process, ProcessState, Thread, ThreadState};
use crate::kprintln;
use alloc::sync::Arc;

#[path = "syscall_channel.rs"]
mod channel;

#[path = "syscall_probe.rs"]
mod probe;

pub(crate) const SYS_YIELD: usize = 0;
pub(crate) const SYS_EXIT: usize = 1;
pub(crate) const SYS_HANDLE_CLOSE: usize = 2;
pub(crate) const SYS_HANDLE_DUPLICATE: usize = 3;
pub(crate) const SYS_CAP_INSPECT: usize = 4;
pub(crate) const SYS_CAP_ATTENUATE: usize = 5;
pub(crate) const SYS_CAP_REVOKE: usize = 6;
pub(crate) const SYS_CHAN_CREATE: usize = 7;
pub(crate) const SYS_CHAN_SEND: usize = 8;
pub(crate) const SYS_CHAN_RECV: usize = 9;

#[cfg(all(feature = "user-write-probe", feature = "user-execute-probe"))]
compile_error!("user-write-probe and user-execute-probe must run separately");

pub(crate) const USER_PC: usize = 0x0000_0010_0000_0000;
pub(crate) const USER_STACK: usize = 0x0000_0020_0000_0000;
pub(crate) const STACK_SIZE: usize = 16384;

struct Bootstrap {
    _root: Arc<SpinLock<Job>>,
    process: Arc<SpinLock<Process>>,
    thread: Arc<SpinLock<Thread>>,
    _code: Vmo,
    probe: probe::Observer,
    _fixtures: Option<probe::Fixtures>,
    isolation: bool,
}

// shortcut: one resident task, replace with per-CPU current-task state before multitask scheduling.
static BOOTSTRAP: SpinLock<Option<Bootstrap>> = SpinLock::new(None);

pub fn run(root: Arc<SpinLock<Job>>, memory: bitmap::MemoryStats) -> ! {
    address_space_lifecycle(memory);
    probe::self_test();
    channel::self_test();
    let process = root
        .lock()
        .create_process("user_probe")
        .expect("create user process");
    let thread = process
        .lock()
        .create_named_thread("user_probe/main", USER_PC, STACK_SIZE)
        .expect("create user thread");
    let isolation = cfg!(feature = "user-isolation-probe");
    let probe = if isolation {
        arch::isolation_probe()
    } else {
        arch::user_probe()
    };
    let mut code = Vmo::create(probe.len()).expect("allocate user code");
    code.write(0, probe).expect("copy user probe");
    let stack = Vmo::create(STACK_SIZE).expect("allocate user stack");
    let kernel_sp = {
        let mut task = thread.lock();
        let physical = task.kernel_stack().frames().get(0).unwrap();
        let top = crate::kernel::memory::phys_map::phys_to_virt(physical)
            .expect("kernel stack alias")
            + task.kernel_stack().size();
        task.set_user_stack(stack);
        task.set_state(ThreadState::Running);
        top
    };
    let mut proc = process.lock();
    assert_eq!(proc.handle_table().lock().count(), 0);
    let fixtures = if isolation {
        None
    } else {
        Some(probe::Fixtures::delegate(&proc))
    };
    proc.set_state(ProcessState::Running);
    let space = proc.address_space_mut();
    unsafe {
        let code_flags = if cfg!(feature = "user-execute-probe") {
            MapFlags::user_ro()
        } else {
            MapFlags::user_rx()
        };
        space
            .map_vmo(USER_PC, code.frames(), code_flags)
            .expect("map user code");
        let task = thread.lock();
        space
            .map_vmo(
                USER_STACK,
                task.user_stack().unwrap().frames(),
                MapFlags::user_rw(),
            )
            .expect("map user stack");
    }
    drop(proc);
    *BOOTSTRAP.lock() = Some(Bootstrap {
        _root: root,
        process: process.clone(),
        thread,
        _code: code,
        probe: probe::Observer::new(),
        _fixtures: fixtures,
        isolation,
    });
    kprintln!(
        "[user] entering {} probe",
        if isolation { "isolation" } else { "roundtrip" }
    );
    unsafe {
        process.lock().address_space().activate();
    }
    // All owners remain in BOOTSTRAP and no lock survives the privilege switch.
    unsafe { arch::drop_to_user(USER_PC, USER_STACK + STACK_SIZE, kernel_sp) }
}

fn address_space_lifecycle(memory: bitmap::MemoryStats) {
    let before = bitmap::stats().free_pages;
    let mut space = ArchAddressSpace::new().expect("create disposable address space");
    let _ = (&mut space, memory);
    unsafe {
        let original: usize;
        #[cfg(target_arch = "riscv64")]
        core::arch::asm!("csrr {}, satp", out(reg) original);
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, cr3", out(reg) original);
        space.activate();
        core::hint::black_box(space.root_paddr());
        #[cfg(target_arch = "riscv64")]
        core::arch::asm!("csrw satp, {}", "sfence.vma", in(reg) original);
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov cr3, {}", in(reg) original);
    }
    drop(space);
    assert_eq!(
        bitmap::stats().free_pages,
        before,
        "address-space table leak"
    );
    kprintln!("[mem]   address-space activation and teardown: ok");
}

fn cap_errno(error: CapError) -> isize {
    match error {
        CapError::InvalidHandle => -9,
        CapError::InsufficientRights => -13,
        CapError::TableFull => -24,
        CapError::Revoked => -116,
        CapError::InvalidRights => -22,
        CapError::OutOfMemory => -12,
        CapError::NodeCapacity => -28,
    }
}

/// Execute returning syscalls using only the trusted calling process.
/// No test bookkeeping or resident probe lock participates in object operations.
pub(crate) fn dispatch_for_process(
    process: &Arc<SpinLock<Process>>,
    nr: usize,
    args: [usize; 6],
) -> isize {
    if nr == SYS_YIELD {
        return 0;
    }
    if matches!(nr, SYS_CHAN_CREATE | SYS_CHAN_SEND | SYS_CHAN_RECV) {
        return channel::dispatch(process, nr, args);
    }
    if !matches!(
        nr,
        SYS_HANDLE_CLOSE
            | SYS_HANDLE_DUPLICATE
            | SYS_CAP_INSPECT
            | SYS_CAP_ATTENUATE
            | SYS_CAP_REVOKE
    ) {
        return -38;
    }
    let handle = match u32::try_from(args[0]) {
        Ok(handle) => handle,
        Err(_) => return -9,
    };
    match nr {
        SYS_HANDLE_CLOSE => {
            let removed = {
                let process = process.lock();
                let mut table = process.handle_table().lock();
                table.remove(handle)
            };
            match removed {
                Ok(node) => {
                    // Last owners can destroy objects that acquire other kernel locks.
                    drop(node);
                    0
                }
                Err(error) => cap_errno(error),
            }
        }
        SYS_HANDLE_DUPLICATE => {
            let result = {
                let process = process.lock();
                let mut table = process.handle_table().lock();
                table.duplicate(handle)
            };
            result.map(|h| h as isize).unwrap_or_else(cap_errno)
        }
        SYS_CAP_INSPECT => {
            let process = process.lock();
            let info = process.handle_table().lock().inspect(handle);
            match info {
                Ok(info) => {
                    // The process lock pins resident mappings; retained VMO owners pin RAM aliases.
                    match unsafe {
                        copy_to_user(process.address_space(), args[1], &info.to_le_bytes())
                    } {
                        Ok(()) => 0,
                        Err(_) => -14,
                    }
                }
                Err(error) => cap_errno(error),
            }
        }
        SYS_CAP_ATTENUATE => {
            let (source, rights) = {
                let process = process.lock();
                let table = process.handle_table().lock();
                let source = match table.lookup(handle) {
                    Ok(source) => source,
                    Err(error) => return cap_errno(error),
                };
                if !source.rights().contains(Rights::DUPLICATE) {
                    return -13;
                }
                let rights = match u32::try_from(args[1]).ok().and_then(Rights::from_bits) {
                    Some(rights) => rights,
                    None => return -22,
                };
                if !source.rights().contains(rights) {
                    return -13;
                }
                if table.is_full() {
                    return -24;
                }
                (Arc::clone(source), rights)
            };
            let child = match source.attenuate(rights) {
                Ok(child) => child,
                Err(error) => return cap_errno(error),
            };
            let inserted = {
                let process = process.lock();
                let mut table = process.handle_table().lock();
                if source.is_valid() {
                    table.insert(child)
                } else {
                    Err((CapError::Revoked, child))
                }
            };
            match inserted {
                Ok(handle) => handle as isize,
                Err((error, child)) => {
                    drop(child);
                    cap_errno(error)
                }
            }
        }
        SYS_CAP_REVOKE => {
            let node = {
                let process = process.lock();
                let table = process.handle_table().lock();
                let node = match table.lookup(handle) {
                    Ok(node) => node,
                    Err(error) => return cap_errno(error),
                };
                if !node.rights().contains(Rights::MANAGE) {
                    return -13;
                }
                Arc::clone(node)
            };
            node.revoke();
            0
        }
        _ => unreachable!(),
    }
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn observe_x86_flags(nr: usize, flags: u64) {
    let bootstrap = BOOTSTRAP.lock();
    if let Some(task) = bootstrap.as_ref() {
        task.probe.x86_flags(nr, flags);
    }
}

pub fn dispatch(nr: usize, args: [usize; 6]) -> isize {
    let (process, thread) = {
        let bootstrap = BOOTSTRAP.lock();
        let task = bootstrap
            .as_ref()
            .expect("syscall without current user task");
        (Arc::clone(&task.process), Arc::clone(&task.thread))
    };
    if nr == SYS_EXIT {
        thread.lock().set_state(ThreadState::Dead);
        let count = process.lock().handle_table().lock().count();
        {
            let bootstrap = BOOTSTRAP.lock();
            let task = bootstrap.as_ref().unwrap();
            task.probe.finish(args[0], count, task.isolation);
        }
        kprintln!("[FERBER_TEST] privilege_switch_roundtrip: PASS");
        kprintln!("[FERBER_TEST] handle_syscalls: PASS");
        kprintln!("[FERBER_TEST] capability_mutation: PASS");
        kprintln!("[FERBER_TEST] channel_syscalls: PASS");
        halt()
    }
    let result = dispatch_for_process(&process, nr, args);
    {
        let mut bootstrap = BOOTSTRAP.lock();
        let task = bootstrap.as_mut().unwrap();
        task.probe.observe(nr, args, result, task.isolation);
    }
    result
}

pub fn user_fault(pc: usize, addr: usize, cause: usize) -> ! {
    let bootstrap = BOOTSTRAP.lock();
    let task = bootstrap.as_ref().expect("fault without current user task");
    assert!(
        task.isolation,
        "unexpected user fault pc={pc:#x} address={addr:#x}"
    );
    let offset = if cfg!(feature = "user-execute-probe") {
        0
    } else {
        arch::isolation_fault_offset()
    };
    assert_eq!(pc, USER_PC + offset);
    assert_eq!(addr, arch::isolation_address());
    #[cfg(target_arch = "riscv64")]
    {
        let expected = if cfg!(feature = "user-write-probe") {
            15
        } else if cfg!(feature = "user-execute-probe") {
            12
        } else {
            13
        };
        assert_eq!(cause, expected);
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(cause, 14);
    }
    assert_eq!(task.process.lock().handle_table().lock().count(), 0);
    task.thread.lock().set_state(ThreadState::Dead);
    drop(bootstrap);
    let marker = if cfg!(feature = "user-write-probe") {
        "privilege_write_rx"
    } else if cfg!(feature = "user-execute-probe") {
        "privilege_execute_nx"
    } else {
        "privilege_isolation"
    };
    kprintln!("[FERBER_TEST] {}: PASS", marker);
    halt()
}

fn halt() -> ! {
    loop {
        #[cfg(target_arch = "riscv64")]
        unsafe {
            core::arch::asm!("wfi")
        };
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("cli", "hlt")
        };
    }
}
