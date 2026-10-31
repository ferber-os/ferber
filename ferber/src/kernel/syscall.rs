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
use crate::kernel::scheduler;
use crate::kernel::sync::SpinLock;
use crate::kernel::task::{Job, Process, Thread};
use crate::kprintln;
use alloc::sync::Arc;

#[path = "syscall_channel.rs"]
mod channel;

#[path = "syscall_vmo.rs"]
mod vmo;

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
pub(crate) const SYS_CHAN_RECV_WAIT: usize = 10;
pub(crate) const SYS_VMO_CREATE: usize = 11;
pub(crate) const SYS_VMO_READ: usize = 12;
pub(crate) const SYS_VMO_WRITE: usize = 13;
pub(crate) const SYS_VMO_MAP: usize = 14;
pub(crate) const SYS_VMO_UNMAP: usize = 15;
pub(crate) const SYS_VMO_SEAL: usize = 16;
pub(crate) const SYS_CHAN_WAIT_MANY: usize = 23;

#[cfg(all(feature = "user-write-probe", feature = "user-execute-probe"))]
compile_error!("user-write-probe and user-execute-probe must run separately");
#[cfg(all(
    feature = "userspace-elf-probe",
    any(
        feature = "user-isolation-probe",
        feature = "cooperative-deadlock-probe",
        feature = "x86-exception-probe",
    )
))]
compile_error!("userspace-elf-probe must run separately from other probe scenarios");

#[cfg(all(
    feature = "init",
    any(
        feature = "userspace-elf-probe",
        feature = "user-isolation-probe",
        feature = "cooperative-deadlock-probe",
        feature = "x86-exception-probe",
    )
))]
compile_error!("init must run separately from other probe scenarios");

#[cfg(all(
    feature = "userspace-vmo-probe",
    any(
        feature = "init",
        feature = "userspace-elf-probe",
        feature = "user-isolation-probe",
        feature = "cooperative-deadlock-probe",
        feature = "x86-exception-probe",
    )
))]
compile_error!("userspace-vmo-probe must run separately from other probe scenarios");

#[cfg(all(
    feature = "userspace-vmo-map-probe",
    any(
        feature = "init",
        feature = "userspace-elf-probe",
        feature = "userspace-vmo-probe",
        feature = "user-isolation-probe",
        feature = "cooperative-deadlock-probe",
        feature = "x86-exception-probe",
    )
))]
compile_error!("userspace-vmo-map-probe must run separately from other probe scenarios");
#[cfg(all(
    feature = "user-vmo-map-write-probe",
    feature = "user-vmo-map-execute-probe"
))]
compile_error!("VMO mapping write and execute fault probes must run separately");

#[cfg(all(
    feature = "userspace-vmo-seal-probe",
    any(
        feature = "init",
        feature = "userspace-elf-probe",
        feature = "userspace-vmo-probe",
        feature = "userspace-vmo-map-probe",
        feature = "user-isolation-probe",
        feature = "user-write-probe",
        feature = "user-execute-probe",
        feature = "cooperative-deadlock-probe",
        feature = "x86-exception-probe",
    )
))]
compile_error!("userspace-vmo-seal-probe must run separately from other probe scenarios");

#[cfg(all(
    feature = "userspace-job-accounting-probe",
    any(
        feature = "init",
        feature = "userspace-elf-probe",
        feature = "userspace-vmo-probe",
        feature = "userspace-vmo-map-probe",
        feature = "userspace-vmo-seal-probe",
        feature = "user-isolation-probe",
        feature = "cooperative-deadlock-probe",
        feature = "x86-exception-probe",
    )
))]
compile_error!("userspace-job-accounting-probe must run separately from other scenarios");

pub(crate) const USER_PC: usize = 0x0000_0010_0000_0000;
pub(crate) const USER_STACK: usize = 0x0000_0020_0000_0000;
pub(crate) const STACK_SIZE: usize = 16384;

struct ProbeState {
    observer: probe::Observer,
    _fixtures: Option<probe::Fixtures>,
    isolation: bool,
}

static PROBE: SpinLock<Option<ProbeState>> = SpinLock::new(None);

fn create_task(
    root: &Arc<SpinLock<Job>>,
    name: &'static str,
    bytes: &[u8],
    flags: MapFlags,
    role: Option<u64>,
) -> (Arc<SpinLock<Process>>, Arc<SpinLock<Thread>>) {
    let process = root
        .lock()
        .create_process(name)
        .expect("create user process");
    let thread = process
        .lock()
        .create_named_thread(name, USER_PC, STACK_SIZE)
        .expect("create user thread");
    let mut code = Vmo::create(bytes.len()).expect("allocate user code");
    code.write(0, bytes).expect("copy user code");
    let mut stack = Vmo::create(STACK_SIZE).expect("allocate user stack");
    if let Some(role) = role {
        stack.write(0, &role.to_le_bytes()).unwrap();
        stack
            .write(16, &(0x7400_0000u64 | role).to_le_bytes())
            .unwrap();
    }
    {
        let mut proc = process.lock();
        assert_eq!(proc.handle_table().lock().count(), 0, "empty birth table");
        unsafe {
            proc.address_space_mut()
                .map_vmo(USER_PC, code.frames(), flags)
                .expect("map user code");
            proc.address_space_mut()
                .map_vmo(USER_STACK, stack.frames(), MapFlags::user_rw())
                .expect("map user stack");
        }
        proc.retain_backing(code);
    }
    {
        let mut task = thread.lock();
        task.set_user_stack(stack);
        task.set_user_stack_mapping(USER_STACK);
    }
    (process, thread)
}

fn cooperative_batch(root: &Arc<SpinLock<Job>>) -> scheduler::Stats {
    use crate::kernel::capabilities::{ObjectType, CAP_NODE_ALLOCATOR};
    use crate::kernel::ipc::channel::Channel;
    {
        let (first, second) = Channel::create().expect("cooperative channel");
        for (role, endpoint) in [first, second].into_iter().enumerate() {
            let (process, thread) = create_task(
                root,
                "cooperative",
                arch::cooperative_probe(),
                MapFlags::user_rx(),
                Some(if cfg!(feature = "cooperative-deadlock-probe") {
                    0
                } else {
                    role as u64
                }),
            );
            let node = CAP_NODE_ALLOCATOR
                .alloc_root(endpoint, ObjectType::Channel, channel::ENDPOINT_RIGHTS)
                .expect("cooperative endpoint");
            assert_eq!(
                process
                    .lock()
                    .handle_table()
                    .lock()
                    .insert(node)
                    .unwrap_or_else(|_| panic!("delegate endpoint")),
                0
            );
            scheduler::admit(process, thread).expect("admit cooperative task");
        }
    }
    scheduler::run()
}

pub fn run(root: Arc<SpinLock<Job>>, memory: bitmap::MemoryStats) -> ! {
    super::memory::accounting_tests::run();
    address_space_lifecycle(memory);
    probe::self_test();
    channel::self_test();
    vmo::self_test();
    scheduler::init();
    scheduler::self_test(&root);
    if cfg!(feature = "userspace-job-accounting-probe") {
        #[cfg(feature = "userspace-job-accounting-probe")]
        super::job_accounting_probe::run(&root);
        halt();
    }
    if cfg!(feature = "userspace-vmo-seal-probe") {
        #[cfg(feature = "userspace-vmo-seal-probe")]
        super::vmo_seal_probe::run(&root);
        halt();
    }
    if cfg!(feature = "userspace-vmo-map-probe") {
        #[cfg(feature = "userspace-vmo-map-probe")]
        super::vmo_map_probe::run(&root);
        halt();
    }
    if cfg!(feature = "init") {
        #[cfg(feature = "init")]
        super::init::run(&root);
    }
    if cfg!(feature = "userspace-elf-probe") {
        #[cfg(feature = "userspace-elf-probe")]
        super::elf_probe::run(&root);
        halt();
    }
    if cfg!(feature = "userspace-vmo-probe") {
        #[cfg(feature = "userspace-vmo-probe")]
        super::vmo_probe::run(&root);
        halt();
    }
    let isolation = cfg!(feature = "user-isolation-probe");
    {
        let bytes = if isolation {
            arch::isolation_probe()
        } else {
            arch::user_probe()
        };
        let flags = if cfg!(feature = "user-execute-probe") {
            MapFlags::user_ro()
        } else {
            MapFlags::user_rx()
        };
        let (process, thread) = create_task(&root, "user_probe", bytes, flags, None);
        let fixtures = if isolation {
            None
        } else {
            Some(probe::Fixtures::delegate(&process.lock()))
        };
        *PROBE.lock() = Some(ProbeState {
            observer: probe::Observer::new(),
            _fixtures: fixtures,
            isolation,
        });
        scheduler::admit(process, thread).expect("admit user probe");
    }
    kprintln!(
        "[user] entering {} probe",
        if isolation { "isolation" } else { "roundtrip" }
    );
    let legacy = scheduler::run();
    assert_eq!(legacy.reaped, 1);
    kprintln!("[FERBER_TEST] privilege_switch_roundtrip: PASS");
    kprintln!("[FERBER_TEST] handle_syscalls: PASS");
    kprintln!("[FERBER_TEST] capability_mutation: PASS");
    kprintln!("[FERBER_TEST] channel_syscalls: PASS");
    // Warm persistent registry/queue capacity before checking exact task reclamation.
    cooperative_batch(&root);
    let frames = bitmap::stats().free_pages;
    let nodes = crate::kernel::capabilities::CAP_NODE_ALLOCATOR.live_count();
    let processes = root.lock().process_count();
    for _ in 0..3 {
        let stats = cooperative_batch(&root);
        assert!(stats.yields >= 8 && stats.switches > stats.yields);
        assert!(stats.parks >= 4 && stats.wakes >= 4);
        assert_eq!(stats.reaped, 2);
        assert_eq!(scheduler::task_count(), 0);
        assert_eq!(scheduler::ready_count(), 0);
        assert_eq!(root.lock().process_count(), processes);
        assert_eq!(
            crate::kernel::capabilities::CAP_NODE_ALLOCATOR.live_count(),
            nodes
        );
        assert_eq!(
            bitmap::stats().free_pages,
            frames,
            "resident task frame leak"
        );
    }
    kprintln!("[FERBER_TEST] cooperative_scheduling: PASS");
    kprintln!("[FERBER_TEST] blocking_channel_receive: PASS");
    kprintln!("[FERBER_TEST] task_reclamation: PASS");
    halt()
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

pub(crate) fn cap_errno(error: CapError) -> isize {
    match error {
        CapError::InvalidHandle => -9,
        CapError::InsufficientRights => -13,
        CapError::TableFull => -24,
        CapError::Revoked => -116,
        CapError::InvalidRights => -22,
        CapError::OutOfMemory => -12,
        CapError::NodeCapacity | CapError::JobQuota => -28,
    }
}

pub(crate) fn allocation_errno(error: crate::kernel::memory::accounting::AllocError) -> isize {
    match error {
        crate::kernel::memory::accounting::AllocError::Quota => -28,
        crate::kernel::memory::accounting::AllocError::OutOfMemory => -12,
    }
}

/// Execute returning syscalls using only the trusted calling process.
/// No test bookkeeping or resident probe lock participates in object operations.
pub(crate) fn dispatch_for_process(
    process: &Arc<SpinLock<Process>>,
    nr: usize,
    args: [usize; 6],
) -> isize {
    let owner = process.lock().budget().clone();
    crate::kernel::memory::accounting::with_budget(&owner, || dispatch_in_budget(process, nr, args))
}

fn dispatch_in_budget(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    if (17..=22).contains(&nr) {
        return super::syscall_task::dispatch(process, nr, args);
    }
    if nr == SYS_YIELD {
        return 0;
    }
    if matches!(nr, SYS_CHAN_CREATE | SYS_CHAN_SEND | SYS_CHAN_RECV) {
        return channel::dispatch(process, nr, args);
    }
    if matches!(
        nr,
        SYS_VMO_CREATE | SYS_VMO_READ | SYS_VMO_WRITE | SYS_VMO_MAP | SYS_VMO_UNMAP | SYS_VMO_SEAL
    ) {
        return vmo::dispatch(process, nr, args);
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
                    let process_id = process.lock().id();
                    scheduler::cancel_closed(process_id, args[0]);
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
    if let Some(probe) = PROBE.lock().as_ref() {
        probe.observer.x86_flags(nr, flags);
    }
}

pub fn dispatch(nr: usize, args: [usize; 6]) -> isize {
    if nr == SYS_EXIT {
        {
            let process = scheduler::current_process();
            let handles = process.lock().handle_table().lock().count();
            #[cfg(feature = "userspace-job-accounting-probe")]
            super::job_accounting_probe::finish(args[0], handles);
            #[cfg(feature = "userspace-elf-probe")]
            super::elf_probe::finish(args[0], handles);
            #[cfg(feature = "userspace-vmo-probe")]
            super::vmo_probe::finish(args[0], handles);
            #[cfg(feature = "userspace-vmo-map-probe")]
            super::vmo_map_probe::finish(args[0], handles);
            #[cfg(feature = "userspace-vmo-seal-probe")]
            super::vmo_seal_probe::finish(args[0], handles);
            let state = PROBE.lock().take();
            if let Some(state) = state {
                state.observer.finish(args[0], handles, state.isolation);
            } else {
                #[cfg(feature = "init")]
                super::init::record_exit(args[0]);
            }
        }
        scheduler::exit()
    }
    let result = if nr == SYS_YIELD {
        scheduler::sched_yield();
        0
    } else if nr == SYS_CHAN_WAIT_MANY {
        let input = {
            let process = scheduler::current_process();
            channel::wait_many_input(&process, args)
        };
        match input {
            Err(error) => error,
            Ok((handles, count)) => loop {
                if let Some(error) = scheduler::wait_error() {
                    scheduler::finish_wait();
                    break error;
                }
                let (attempt, generation) = {
                    let (token, generation) = match scheduler::prepare_wait() {
                        Ok(wait) => wait,
                        Err(error) => break error,
                    };
                    let process = scheduler::current_process();
                    (
                        channel::wait_many_attempt(&process, &handles[..count], &token, generation),
                        generation,
                    )
                };
                match attempt {
                    channel::WaitManyAttempt::Complete(result) => {
                        scheduler::finish_wait();
                        break result;
                    }
                    channel::WaitManyAttempt::Park { first, extra } => {
                        scheduler::park_many(
                            first.handle,
                            generation,
                            first.node,
                            first.registration,
                            extra,
                        );
                    }
                }
            },
        }
    } else if nr == SYS_CHAN_RECV_WAIT {
        loop {
            if let Some(error) = scheduler::wait_error() {
                scheduler::finish_wait();
                break error;
            }
            let (attempt, generation) = {
                let (token, generation) = match scheduler::prepare_wait() {
                    Ok(wait) => wait,
                    Err(error) => break error,
                };
                let process = scheduler::current_process();
                (
                    channel::blocking_attempt(&process, args, &token, generation),
                    generation,
                )
            };
            match attempt {
                channel::ReceiveAttempt::Complete(result) => {
                    scheduler::finish_wait();
                    break result;
                }
                channel::ReceiveAttempt::Park { node, registration } => {
                    scheduler::park(args[0], generation, node, registration)
                }
            }
        }
    } else {
        let process = scheduler::current_process();
        dispatch_for_process(&process, nr, args)
    };
    #[cfg(feature = "init")]
    super::init::observe(nr, args, result);
    if let Some(probe) = PROBE.lock().as_mut() {
        probe.observer.observe(nr, args, result, probe.isolation);
    }
    #[cfg(feature = "userspace-job-accounting-probe")]
    super::job_accounting_probe::observe(nr, result);
    #[cfg(feature = "userspace-elf-probe")]
    super::elf_probe::observe(nr, result);
    #[cfg(feature = "userspace-vmo-probe")]
    super::vmo_probe::observe(nr, result);
    #[cfg(feature = "userspace-vmo-map-probe")]
    super::vmo_map_probe::observe(nr, result);
    #[cfg(feature = "userspace-vmo-seal-probe")]
    super::vmo_seal_probe::observe(nr, result);
    result
}

pub fn user_fault(pc: usize, addr: usize, cause: usize) -> ! {
    if cfg!(feature = "user-vmo-seal-write-probe") {
        #[cfg(feature = "user-vmo-seal-write-probe")]
        super::vmo_seal_probe::fault(pc, addr, cause);
    }
    if cfg!(any(
        feature = "user-vmo-map-write-probe",
        feature = "user-vmo-map-execute-probe"
    )) {
        #[cfg(any(
            feature = "user-vmo-map-write-probe",
            feature = "user-vmo-map-execute-probe"
        ))]
        super::vmo_map_probe::fault(pc, addr, cause);
    }
    let state = PROBE.lock();
    let task = state.as_ref().expect("fault without current user task");
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
    assert_eq!(
        scheduler::current_process()
            .lock()
            .handle_table()
            .lock()
            .count(),
        0
    );
    drop(state);
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

pub(crate) fn halt() -> ! {
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
