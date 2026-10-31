# Verification and demonstrated behavior

Ferber checks its contracts at several levels. Host tests exercise production logic with
selected host substitutes. QEMU boots exercise native page tables, privilege transitions
and execution on the two supported architectures. Physical x86-64 boots add firmware and
hardware evidence for the particular build and configuration tested. None of these is a
proof of general correctness.

## Choose the check that covers the behavior

Run commands from the repository root after following the [running
guide](../guides/running.md). A script's presence describes a check available to run; it
does not establish that a particular revision passed it.

| Area | Host entry points | What they cover |
| --- | --- | --- |
| Memory discovery | `bash scripts/memory-tests.sh` | Sparse extents, firmware maps, reservations and allocator invariants |
| Channel core | `bash scripts/channel-tests.sh` | Opposite-direction sends, concurrent registration, queue bounds and deterministic allocation failures |
| Task construction | `bash scripts/task-tests.sh` | Draft lifecycle, delegation, authority, quota denial, retry and failed construction cleanup |
| Scheduler | `bash scripts/scheduler-tests.sh` | Wait generations, weak registrations and rooted versus disconnected graphs |
| Job accounting | `bash scripts/job-accounting-tests.sh` | Ancestry limits, original owners, fragmentation, teardown and failure accounting |
| VMO operations | `bash scripts/vmo-core-tests.sh`; `bash scripts/vmo-syscall-tests.sh` | Backing ownership, copy validation, rights and transaction failures |
| Page tables | `bash scripts/riscv-paging-tests.sh`; `bash scripts/x86-paging-tests.sh` | Production walkers, permissions, rollback and table ownership |
| ELF | `bash scripts/elf-tests.sh`; `bash scripts/elf-load-tests.sh` | Static-image parser and loader behavior, initialized bytes/BSS, guards and setup failures |
| Boot delivery | `bash scripts/uefi-payload-tests.sh`; `bash scripts/boot-bundle-tests.sh` | Payload handling, envelope validation and service records |
| Startup | `bash scripts/init-protocol-tests.sh` | Exact bootstrap records, versions, counts and reserved fields |
| Archive/filesystem | `bash scripts/initramfs-tests.sh` | Parser, deterministic tools, safe extraction, filesystem wire/session state and configuration |
| Log acceptance | `bash scripts/smoke-log.sh --self-test` | Exact diagnostics and missing, duplicated or truncated evidence |

Host substitutes do not perform real context switches or prove hardware page protection.
Host concurrency results do not establish production SMP safety.

## Architecture boots require diagnosed outcomes

```sh
bash scripts/riscv-smoke.sh
bash scripts/x86-smoke.sh
bash scripts/x86-iso-smoke.sh init
bash scripts/x86-iso-smoke.sh service-wake
```

The architecture scripts check debug and release configurations, deliberate faults and
finite cleanup. RISC-V additionally exercises parked-secondary-hart configurations. The
ISO script boots through OVMF's CD-ROM path and checks the same release packaging
mechanism used for the published image. Automated x86 images enable debug-port log
capture and need not be byte-identical to a published ISO. The freestanding x86
inspection script requires ET_DYN, no INTERP header, no NEEDED dependencies and only
R_X86_64_RELATIVE relocations.

Emulator serial output goes directly to a file so process termination cannot lose
stdio-buffered logs. Success requires the expected markers or exact expected fault
diagnostic. A timeout, unrelated panic or silent reboot is not success.

## Normal resident probes exercise the syscall boundary

The default resident probes start from empty process handle tables and receive only
explicitly delegated test capabilities. They check returning syscalls, rights denial,
malformed buffers, revocation, aliases, transactional transfers, cooperative scheduling,
blocking receive and cleanup. Repeated runs restore frame/node baselines and leave no
registered tasks after exit.

A successful ordinary boot requires the memory marker plus exactly one of each:

```text
[FERBER_TEST] privilege_switch_roundtrip: PASS
[FERBER_TEST] handle_syscalls: PASS
[FERBER_TEST] capability_mutation: PASS
[FERBER_TEST] channel_syscalls: PASS
[FERBER_TEST] cooperative_scheduling: PASS
[FERBER_TEST] blocking_channel_receive: PASS
[FERBER_TEST] task_reclamation: PASS
```

The `memory_verification` check walks discovered extents and checks alias translation,
kernel code/stack permissions, supervisor isolation and allocator accounting. It writes
only an ordinary owned allocation, tests zeroing, mapping, user copies and release, then
checks accounting restoration. It is representative backing verification, not an
exhaustive destructive RAM test or a capacity benchmark. No threshold-specific
over-4-GiB build is required.

## ELF markers follow actual program execution

The standalone `userspace-elf-probe` mode enters an independently linked Rust
executable. It checks initialized read-only/writable data, multi-page zeroed BSS, stack
alignment, unknown-syscall result -38, yield with preserved state and exit. Its startup
record identifies an explicitly installed nonzero bootstrap slot. The kernel prequeues
`ferber:1`; the fixture receives it through syscall 9 and sends `ready-v1` through
syscall 8. This finite test schedules no service peer.

After one warmup and three measured runs, frame/node/process baselines must be restored
and the scheduler empty. The required markers are `memory_verification`,
`userspace_elf`, `userspace_bootstrap_channel` and `boot_payload`, each with the
`[FERBER_TEST]` prefix and PASS result. Loading or mapping alone cannot satisfy them.
Historical release checkpoints had narrower criteria; consult their notes rather than
applying current markers retroactively.

## Init verifies every required child before declaring idle

The normal init path launches initfs and configured services. Actual READY and PING/PONG
sends are observed against distinct live process identities and their exact endpoints.
The initial challenge is 74. Replies from before challenge issuance cannot supply later
evidence. Aggregate readiness must match the observed and live child count.

Before the final resident run, finite shutdown runs warm capacities and check restored
physical frames, capability nodes and root process count. SHUTDOWN is forwarded to all
children; children exit zero and init exits with expected status one. Cleanup occurs
from the dispatcher stack after leaving retired task state.

The resident path keeps those processes alive and requires:

```text
[FERBER_TEST] memory_verification: PASS
[FERBER_TEST] boot_payload: PASS
[FERBER_TEST] service_launch: PASS
[FERBER_TEST] multiservice_idle: PASS
[FERBER_TEST] init_ready: PASS
[FERBER_TEST] init_waiting: PASS
```

The `service-wake-probe` feature registers a software source, queues challenge 75 after
the first valid idle state, and requires fresh replies from every child before
`software_wake: PASS`. This demonstrates cooperative software resumption, not
interrupt-driven wakeup or SMP. The normal hardware image does not enable that dedicated
probe.

Startup regressions include malformed messages, missing/invalid service images, replay
attempts, unexpected exits and disconnected waits. Malformed filesystem clients and
flooded reply queues must lose only their own session, while other clients continue.
Integrated allocation-failure injection during multi-service startup remains a coverage
gap despite task/allocator host fault-injection tests.

## Memory probes distinguish access from authority

Separate VMO byte and mapping programs verify copy bounds, aliasing, rights, backing
lifetime, mapping rollback and final accounting restoration. Their normal markers
include `vmo_syscalls`, `vmo_mapping` or `job_accounting`, plus memory and boot-payload
evidence for their respective configurations.

The sealing fixture writes architecture-specific leaf functions, seals and maps them RX,
and executes them. Another process's writable alias must block sealing until exact
unmap. The test checks denied writes, retained mappings and executable ownership after
handle closure/revocation. Replacing sealed function A with separately prepared function
B at the same virtual address verifies a changed result, exercising local
instruction-fetch synchronization.

A warmup and three measured batches restore frame/node/process/scheduler and
backing-budget baselines before `vmo_sealing: PASS`. A separate deliberate store through
sealed RX memory requires a matching user fault before `vmo_sealing_write_rx: PASS`; the
fault halt itself does not prove reclamation.

## Fault tests require the intended fault

Supervisor-read, RX-write and NX-execute builds validate user origin, faulting
instruction, address and architecture cause before their respective
`privilege_isolation`, `privilege_write_rx` and `privilege_execute_nx` markers. Mapping
RO-write and NX-execute fixtures are separate regressions. x86 also retains an
intentional invalid-opcode exception test.

The `cooperative-deadlock-probe` runs receivers without a runnable producer. It must
panic with `cooperative deadlock: live tasks with no runnable producer` and must not
emit cooperative scheduling, blocking receive or reclamation success markers. Legitimate
init waiting cannot turn an unrelated closed wait cycle into a successful idle state.

## Physical evidence belongs to the build tested

Physical x86-64 verification used an MSI B650M-A PRO WIFI system with 32 GiB installed.
The initramfs/initfs implementation reached all six normal init markers with 32,218 MiB
(31.46 GiB) managed RAM across 24 extents. That build used the pre-bump v0.3.0 banner.
This does not establish a physical test of the newly versioned v0.3.1 ISO. The dedicated
software-wake image was not flashed.

Earlier physical measurements belong to their respective releases and firmware maps.
Counts of managed frames or extents are not fixed system capacities. See [the v0.3.1
release record](../releases/Uranium-238-v0.3.1.md) for provenance. No physical RISC-V
result is claimed. Hosted CI is separate evidence and must be attributed to the
particular revision/run observed.
