//! Wolfram syscall ABI. Object operations require explicit handles; yield and
//! exit operate on the current task. Linux-style registers, Wolfram numbers.

use crate::arch::{self, ArchAddressSpace};
use crate::kernel::memory::{
    bitmap,
    paging::{AddressSpace, MapFlags},
    Vmo,
};
use crate::kernel::sync::SpinLock;
use crate::kernel::task::{Job, Process, ProcessState, Thread, ThreadState};
use crate::kprintln;
use alloc::sync::Arc;

const USER_PC: usize = 0x0000_0010_0000_0000;
const USER_STACK: usize = 0x0000_0020_0000_0000;
const STACK_SIZE: usize = 16384;
const ARG_CANARIES: [usize; 6] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];

struct Bootstrap {
    _root: Arc<SpinLock<Job>>,
    process: Arc<SpinLock<Process>>,
    thread: Arc<SpinLock<Thread>>,
    _code: Vmo,
    #[cfg(target_arch = "riscv64")]
    _supervisor: Vmo,
    calls: usize,
    valid_sequence: bool,
    isolation: bool,
}

// shortcut: one resident bootstrap task, replace with per-CPU current-task state in M2.
static BOOTSTRAP: SpinLock<Option<Bootstrap>> = SpinLock::new(None);

pub fn run(root: Arc<SpinLock<Job>>, memory: bitmap::MemoryStats) -> ! {
    address_space_lifecycle(memory);
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
    assert!(probe.len() <= 4096);
    let mut code = Vmo::create(4096).expect("allocate user code");
    code.write(0, probe).expect("copy user probe");
    let stack = Vmo::create(STACK_SIZE).expect("allocate user stack");
    let kernel_sp = {
        let mut task = thread.lock();
        let top = task.kernel_stack().frames().get(0).unwrap() + task.kernel_stack().size();
        task.set_user_stack(stack);
        task.set_state(ThreadState::Running);
        top
    };
    let mut proc = process.lock();
    assert_eq!(proc.handle_table().lock().count(), 0);
    proc.set_state(ProcessState::Running);
    let space = proc.address_space_mut();
    #[cfg(target_arch = "riscv64")]
    let supervisor = unsafe { arch::prepare_user_space(space, memory.start, memory.end) };
    unsafe {
        space
            .map_vmo(USER_PC, code.frames(), MapFlags::user_rx())
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
        #[cfg(target_arch = "riscv64")]
        _supervisor: supervisor,
        calls: 0,
        valid_sequence: true,
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
    #[cfg(target_arch = "riscv64")]
    let backing = unsafe { arch::prepare_user_space(&mut space, memory.start, memory.end) };
    #[cfg(target_arch = "x86_64")]
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
    #[cfg(target_arch = "riscv64")]
    drop(backing);
    assert_eq!(
        bitmap::stats().free_pages,
        before,
        "address-space table leak"
    );
    kprintln!("[mem]   address-space activation and teardown: ok");
}

pub fn dispatch(nr: usize, args: [usize; 6]) -> isize {
    let mut bootstrap = BOOTSTRAP.lock();
    let task = bootstrap
        .as_mut()
        .expect("syscall without current user task");
    assert!(!task.isolation, "syscall during isolation probe");
    assert_eq!(task.thread.lock().state(), ThreadState::Running);
    match nr {
        0 => {
            task.valid_sequence &= task.calls < 2 && args == ARG_CANARIES;
            task.calls = task.calls.saturating_add(1);
            0
        }
        1 => {
            task.thread.lock().set_state(ThreadState::Dead);
            assert_eq!(args[0], 0, "user probe failed");
            assert!(
                task.valid_sequence,
                "user syscall sequence or arguments corrupted"
            );
            assert_eq!(task.calls, 3, "incomplete user roundtrip");
            assert_eq!(task.process.lock().handle_table().lock().count(), 0);
            assert_eq!(task.thread.lock().state(), ThreadState::Dead);
            drop(bootstrap);
            kprintln!("[WOLFRAM_TEST] privilege_switch_roundtrip: PASS");
            halt()
        }
        _ => {
            task.valid_sequence &= nr == 99 && task.calls == 2 && args == ARG_CANARIES;
            task.calls = task.calls.saturating_add(1);
            -38
        }
    }
}

pub fn user_fault(pc: usize, addr: usize, cause: usize) -> ! {
    let bootstrap = BOOTSTRAP.lock();
    let task = bootstrap.as_ref().expect("fault without current user task");
    assert!(
        task.isolation,
        "unexpected user fault pc={pc:#x} address={addr:#x}"
    );
    assert_eq!(pc, USER_PC + arch::isolation_fault_offset());
    #[cfg(target_arch = "riscv64")]
    {
        assert_eq!(addr, 0x80200000);
        assert_eq!(cause, 13);
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(addr, arch::isolation_address());
        assert_eq!(cause, 14);
    }
    assert_eq!(task.process.lock().handle_table().lock().count(), 0);
    task.thread.lock().set_state(ThreadState::Dead);
    drop(bootstrap);
    kprintln!("[WOLFRAM_TEST] privilege_isolation: PASS");
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
