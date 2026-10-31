# Uranium-238 v0.2.5

*Cooperative userspace tasks can now yield, block for channel messages and exit independently. A dispatcher running on permanent kernel mappings reclaims retired stacks and address spaces safely.*

## Yield lets another resident task run

Syscall `0` now yields cooperatively and returns zero when the calling task
resumes. With no other runnable task, it resumes itself. Syscall `1` retires
the calling thread without returning; surviving tasks continue running.

The dispatcher uses the existing callee-saved CPU contexts, VMO-backed kernel
stacks, and per-task trap/syscall frames. It selects each task's address space
and trusted kernel-entry stack before returning to userspace. Both architectures
run the same resident scheduling contract. x86-64 also disables user FSGSBASE
instructions and traps FP/SIMD use to prevent unsaved state crossing task
boundaries. Protection keys are disabled because PKRU is not task state yet.
FP/vector execution remains unsupported.

The ready queue remains a `VecDeque`. Scheduling runs on one active CPU with
interrupts disabled. Tasks must yield, block, or exit before another task runs;
this release adds no timer preemption or SMP execution.

## Blocking receive retains the caller's original authority

Syscall `10` receives with the same six arguments and transactional output
rules as nonblocking receive (`9`). It requires both `READ` and `WAIT`, even
when a message is already queued. Newly created channel endpoints include
`WAIT`; send remains nonblocking.

An empty open queue parks the caller. Message arrival or final peer closure
makes it runnable, and readiness is rechecked before delivery. Another
receiver may consume the message first, causing the caller to wait again.
There is no timeout.

A suspended call retains its original capability identity. Closing its source
slot cancels the call with `-9`; reusing that numeric slot cannot redirect it.
Revoking its node or an ancestor cancels it with `-116`. The dispatcher checks
waits at cooperative boundaries and revalidates authority before delivery.
Cancellation leaves output buffers and queued messages unchanged. Revocation
alone does not close an endpoint; closure still follows its final backing owner.

## Reclamation happens after leaving the exited task

The dispatcher runs on a retained VMO-backed stack under the permanent kernel
root. It detaches exited threads and last-thread processes, then drops their
resources outside scheduler, process, handle-table, and channel locks. It can
therefore release stacks, address spaces, handles, and endpoint backing without
freeing the stack or page tables currently executing the cleanup.

Repeated resident runs check restored physical-frame and capability-node
baselines and an empty scheduler registry. The harness halts after all tasks
exit and verification completes. If live tasks are all blocked without a
runnable producer, it reports a deadlock instead of success.

Normal boots now require eight verification markers with the `[FERBER_TEST]`
prefix: `memory_verification`, `privilege_switch_roundtrip`, `handle_syscalls`,
`capability_mutation`, `channel_syscalls`, `cooperative_scheduling`,
`blocking_channel_receive`, and `task_reclamation`, each ending in `: PASS`.

## Validation includes a new physical x86-64 boot

I booted the sprint ISO on physical x86-64 hardware with 32 GiB installed. The
screenshot shows all eight verification markers passing, followed by the
intended clean halt. This validates the normal resident path.
That boot preceded the final `CR4.PKE` hardening and version metadata bump; the
final CPU-state setup was checked in QEMU. No physical RISC-V run or physical
fault/deadlock probe is claimed.

All 11 final-tree RISC-V smoke cases, all 10 x86 UEFI smoke cases, and the
release ISO smoke passed locally: 22 cases total. RISC-V covers debug/release
normal boots at 128 MiB/1 hart and 256 MiB/4 harts, debug/release supervisor
read, RX-write, and NX-execute probes, and debug deadlock. x86 covers debug
normal at 512 MiB, release normal at 1024 MiB, release invalid-opcode at
512 MiB, debug/release supervisor-read, RX-write, and NX-execute at 512 MiB,
and debug deadlock at 512 MiB. The separate ISO case boots the release image
through UEFI.

The separate debug `cooperative-deadlock-probe` requires the exact diagnostic
`cooperative deadlock: live tasks with no runnable producer` and rejects the
three cooperative success markers. A timeout or unrelated panic cannot pass
this scenario. The configured smoke suites contain 11 RISC-V cases, 10 x86
UEFI cases, and one release ISO case.

Both kernel targets and their CI probe configurations, including the deadlock
probe, passed Clippy with `-D warnings`; release UEFI Clippy also passed.
Debug/release x86 ELF checks, 12 channel host tests, and nine memory host tests
passed. These are local results. No hosted CI run is claimed.

Both kernel and loader versions advance to `0.2.5` without dependency upgrades.
The release asset remains `ferber.iso`.

## General userspace and preemption remain future work

The scheduler currently runs kernel-created resident tasks. ELF loading, init,
userspace task creation, VMO and
FastCall syscalls, interrupt-driven wakeup, timer preemption, SMP, FP/vector
state, and concurrent unmapping remain pending. Graph-owning capability
transfer remains rejected until its lifetime policy is defined. Reclamation
guarantees cover the resident task lifecycle exercised here.
