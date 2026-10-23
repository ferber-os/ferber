# Resident Syscall ABI

Ferber uses Linux-style argument registers with its own syscall numbers.
This ABI cooperatively schedules resident user tasks and opt-in static ELF
fixtures and minimal init on one active CPU with interrupts disabled. It does
not provide Linux or musl compatibility, preemption, userspace task creation,
or service launching.

## Calling Convention

| Architecture | Number | Arguments 0–5 | Signed result |
| --- | --- | --- | --- |
| RISC-V 64 | `a7` | `a0`–`a5` | `a0` |
| x86-64 | `rax` | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` | `rax` |

Enter through `ecall` on RISC-V and `syscall` on x86-64. Unused arguments are
ignored. Returning calls preserve general registers other than the result and
the x86 `syscall` architectural clobbers (`rcx`, `r11`). x86 return flags are
sanitized. FP/vector state is outside the resident ABI. x86-64 clears
`CR4.FSGSBASE` and `CR4.PKE`, and sets `CR0.TS` before user execution. This
prevents unsaved FS/GS bases and PKRU state from passing between tasks, and
traps FP/SIMD instructions. Unsupported instructions follow the resident
harness's fatal exception path.

## Operations

| Number | Name | Arguments | Successful result |
| --- | --- | --- | --- |
| `0` | Cooperative yield | None | `0`, when the task resumes |
| `1` | Thread exit | Exit status | Does not return; retires the calling thread |
| `2` | Handle close | Handle | `0` |
| `3` | Handle duplicate | Handle | New handle number |
| `4` | Capability inspect | Handle, output address | `0` |
| `5` | Capability attenuate | Handle, rights mask | New handle number |
| `6` | Capability revoke | Handle | `0` |
| `7` | Channel create | Pair output address | `0`; writes two handles |
| `8` | Channel send | Endpoint handle, bytes address, byte count, handles address, handle count | `0` |
| `9` | Channel receive | Endpoint handle, bytes address, byte capacity, handles address, handle capacity, counts output address | `0` |
| `10` | Blocking channel receive | Same six arguments as `9` | `0` |
| `11` | VMO create | Size, options (`0`) | New handle number |
| `12` | VMO read | Handle, offset, destination, length | `0` |
| `13` | VMO write | Handle, offset, source, length | `0` |

Every handle is resolved through the trusted calling process's table. Processes
are born with zero handles; delegation is explicit. Handle zero is valid.
Handle arguments greater than `u32::MAX` are rejected rather than truncated.

Close releases one table entry without requiring additional rights. A revoked
handle can be closed. Close releases its owned node reference without revoking
aliases. Nodes and their backing are reclaimed when their final owners are gone;
other handles, typed handles, messages, or descendants may retain them. Closing
an already empty slot is an error.

Duplicate requires `DUPLICATE` and inserts an alias to the same node with the
same rights and type. It does not consume a capability-node allocation. Closing
one alias leaves the others usable; revoking the shared node invalidates all
aliases and descendants. The table currently has 256 slots. Vacant numbers can
be reused, so a previously closed number can later refer to another capability.

Inspect requires `INSPECT`. It copies this fixed eight-byte record to the
supplied writable user address, which need not be aligned:

| Byte offset | Field | Encoding |
| --- | --- | --- |
| `0` | Object type | Little-endian `u32` |
| `4` | Rights bitmask | Little-endian `u32` |

Object types are Process `0`, Thread `1`, VMO `2`, Channel `3`, Job `4`, and
Resource `5`. Rights use the bit positions in the
[capability model](capability-model.md#rights). No pointers, internal node IDs,
parent data, or names are exposed, and the object pointer is not dereferenced.

The copy checks the entire range before writing. Null, overflowing, supervisor,
unmapped, or nonwritable destinations fail without changing any output byte.
Mappings/backing remain stable under the current resident-task constraints;
concurrent unmapping and recoverable copy faults remain future work.

## VMO Byte Operations

Create returns fresh zeroed owned RAM, bounded to 16 MiB and charged for its
object lifetime. Read/write require READ/WRITE respectively, copy exactly at
most 64 KiB and preflight complete ranges before staged copying. Physical VMOs
are excluded. See [the VMO byte ABI](vmo-abi.md) for exact rights, errors,
zero-length behavior, the global budget and aliasing guarantees. No mapping,
unmapping or executable publication syscall is added.

## Capability Mutation and Ownership

Attenuate requires `DUPLICATE` and creates a distinct child node with the same
object and an equal or smaller rights mask. The source remains present. Equal
rights still create a child; a zero-rights child is allowed. Mask bits above bit
31 or outside the defined rights set return `-22`; requesting defined rights
absent from the source returns `-13`.

Revoke requires `MANAGE`. It invalidates the selected node, all its aliases,
and all descendants. Ancestors and siblings remain valid. A repeated revoke
returns `-116`; revoked handles can still be closed. Revocation denies future
capability operations. It does not remove existing VMO mappings or reclaim tasks.

Tables, typed handles, and messages own `Arc<CapNode>` references. Each node owns
its parent and object. A live descendant therefore retains its ancestry and
backing even after all ancestor handles close. Validity checks walk ancestors
with Acquire loads; revocation uses one Release store. Dropping an unshared
ancestry chain is iterative rather than recursive.

The node factory allows 1,024 live or reserved nodes globally, including roots
and descendants. Duplicate aliases do not consume this quota. Child allocation
uses fallible heap storage and checked `u64` IDs; failed allocations release
reserved quota. Closing handles can make quota available again. ID exhaustion
is permanent, and IDs are not exposed through inspection.

## Channel IPC

Create writes an eight-byte record containing two little-endian `u32` endpoint
handles. No input handle is required. Both endpoints are created and published
together. Their rights are `READ`, `WRITE`, `WAIT`, `TRANSFER`, `DUPLICATE`,
`INSPECT`, and `MANAGE`; memory and execution rights are absent. Failure leaves the table
and output unchanged.

Send copies at most 65,536 bytes and 64 little-endian `u32` handle numbers into
one message. Empty messages are valid. The endpoint requires `WRITE`; each
payload handle must be valid and carry `TRANSFER`. On success the selected
source slots are removed and their owned node references move into the peer's
queue. Repeated source slot numbers return `-22`. Distinct slots that alias one
node are allowed. Payloads are limited to VMOs and the current leaf Resource
object, with both their type tag and actual backing checked. Channel, Process,
Job, Thread, and other backing return `-22`, including aliases and descendants.
Graph-owning payloads need a lifetime policy before queues can retain them safely.

Nonblocking receive (`9`) requires `READ` and consumes the oldest message only when the supplied
buffers and handle table have room. Its eight-byte counts record contains the
received byte count followed by handle count, both little-endian `u32` values.
The handles buffer receives little-endian `u32` destination slots; unused output
capacity is untouched. Short buffers return `-90` and preserve the message for
retry. A full receiver table returns `-24` with the same preservation guarantee.
Revoked in-flight nodes remain closeable but provide no usable authority.

All nonempty user ranges are checked for overflow, effective user permissions,
and complete mapping before any output changes. Null addresses are allowed for
zero-length buffers. Receive rejects overlapping nonempty byte, handle, and
counts output ranges with `-22`, including distinct virtual ranges aliasing
the same physical bytes. Successful receipt publishes bytes, new handle
slots, and counts together under the resident stable-mapping contract. Every
failed receive leaves user outputs, the queue, and the table unchanged. Every
failed send preserves the sender's exact source slots and queue contents.

Each incoming queue holds up to 64 messages. Send to a full queue and
nonblocking receive from an empty open queue return `-11`. Send to a closed peer returns `-32`; receive drains already queued
messages before returning `-32` for an empty closed queue. Endpoint closure is
observed after its last backing owner is released. Discarded messages release
node references without revoking shared aliases or descendants. Queue state
uses one lock per endpoint pair; discarded messages are destroyed after that
lock is released. Endpoint, message, and queue allocations are fallible.

Blocking receive (`10`) requires both `READ` and `WAIT`, including when a message
is already queued. It uses the same capacities, buffer validation, publication,
and retry rules as `9`. An empty open queue parks the calling thread. Message
arrival or final peer closure makes the waiter runnable; readiness is rechecked
before consuming a message, so another receiver may win and the caller may park
again. There is no timeout and no blocking send.

A suspended receive retains its original capability identity. Closing its source
slot cancels it with `-9`, even if a readiness notification already occurred;
reusing the numeric slot cannot retarget the wait. Revocation of that node or an
ancestor cancels it with `-116`. The dispatcher scans waits at cooperative
boundaries and revalidates authority before resumed delivery. Cancellation does
not change user outputs or consume a message. Revocation denies authority; it
does not itself close the endpoint. Endpoint closure still requires release of
its final backing owner.

## Cooperative Scheduling and Exit

Yield places a running thread behind other ready threads and returns `0` when
it resumes. With no other runnable task, it resumes itself. Kernel continuations
use the architecture's existing callee-saved `CpuContext`; each task retains its
trap/syscall frame on its VMO-backed kernel stack. Address-space and trusted
kernel-entry stack state are selected before entering that task's userspace.

Exit marks the calling thread Dead and never resumes its continuation. The
permanent-root dispatcher runs on a retained VMO-backed stack, so the exited
thread's active stack and tables are not freed while in use. Final thread and
last-thread process resources are detached and destroyed outside scheduler,
process, handle-table, and channel locks. This includes handles whose destruction
closes endpoints or discards queued messages. Surviving tasks continue running.
When all resident tasks exit, the scheduler returns to the kernel harness, which
checks reclamation before halting. Live tasks all blocked without a runnable
wake source are diagnosed as deadlock, not successful completion.
The separate [minimal-init session](minimal-init.md) accepts only its exact,
validated bootstrap receive as terminal waiting. An open peer alone provides
no exemption and no interrupt-driven wakeup guarantee.

Scheduling remains cooperative, single-CPU, and integer-only. Interrupt-driven
wakeup, preemption, SMP, concurrent unmapping, FP/vector state, and user-created
tasks remain outside this ABI. The separately compiled ELF fixture uses this
same syscall table; ELF loading adds no syscall numbers.

## Errors and Validation Order

| Result | Meaning |
| --- | --- |
| `-9` | Invalid handle, including nonzero bits above bit 31, or suspended receive cancelled by source-slot close |
| `-11` | Channel queue is full, or empty with its peer still open |
| `-12` | Capability-node, endpoint, message, queue, VMO frame/metadata/object, or staging allocation failed |
| `-13` | Required right is absent |
| `-14` | User buffer range is invalid or lacks the required user permissions |
| `-22` | Invalid rights mask, options, logical VMO range or backing/type; repeated transfer slot; unsupported payload; or overlapping output bytes |
| `-24` | Handle table is full |
| `-28` | User-VMO backing budget, live/reserved node quota, node IDs, or wait generations exhausted |
| `-32` | Channel peer is closed and no queued message can be received |
| `-38` | Unknown syscall number |
| `-90` | VMO size, byte-copy or message limit exceeded, or receive buffers are too small |
| `-116` | Capability node is revoked |

For duplicate, inspect, attenuate, and revoke, validate the handle, then node
validity (including ancestors), then the required right. Attenuation then checks
mask encoding, the rights subset, table capacity, and node allocation, in that
order; it rechecks source validity before publishing the child. Duplication
checks capacity after source rights; inspection checks output memory after
source rights. Rejected calls leave the table unchanged. Inspection-copy
failures also leave the output unchanged. These error values do not imply Linux ABI compatibility.

For channel send and receive, validate the endpoint handle, node validity,
required right, and actual endpoint backing before other arguments. Send then
checks size limits, reserves byte storage, copies bytes and handle IDs, then
validates each payload in order. It reserves handle and queue storage and checks
peer/queue admission before removing any source slot. Receive
checks peer/queue state, head-message capacities, table space, and all output
ranges before copying or dequeuing. Create validates its output range and two
free slots before allocation, and rechecks them before publication.

## Resident Verification

Normal probes receive explicitly delegated fixture handles after their empty
birth-table check. They exercise permitted and denied operations, invalid and
revoked handles, table exhaustion/recovery, user-copy boundaries, and complete
handle cleanup. Mutation checks cover rights masks, descendant revocation,
ancestor/sibling survival, retained ancestry, and allocation limits/recovery.
Channel checks cover nonblocking message exchange and capability moves, rights
denial, malformed buffers, retry after short buffers, bounded queues, peer
closure, and failed-operation preservation. Probe bookkeeping is separate from
runtime dispatch. Scheduling probes run isolated resident tasks through repeated
yields and blocking receive, including peer closure and cancellation. Repeated
runs must restore the physical-frame and capability-node baselines and leave
the scheduler registry empty after all tasks exit.

`bash scripts/channel-tests.sh` runs host regressions against the channel core
for opposite-direction sends, deterministic allocation failures, full-queue
admission, failed-receive retry, and draining after peer closure. In-kernel
checks use real capability nodes to cover ownership and destructor reentrancy.

The resident test observer diagnoses a nonzero exit status or incomplete
coverage as a failed run, with a kernel panic rather than success markers.

A successful normal boot also requires `memory_verification: PASS` and emits
exactly one of each resident marker:

```text
[FERBER_TEST] privilege_switch_roundtrip: PASS
[FERBER_TEST] handle_syscalls: PASS
[FERBER_TEST] capability_mutation: PASS
[FERBER_TEST] channel_syscalls: PASS
[FERBER_TEST] cooperative_scheduling: PASS
[FERBER_TEST] blocking_channel_receive: PASS
[FERBER_TEST] task_reclamation: PASS
```

Isolation, RX-write, and NX-execute probes remain separate builds without these
handle fixtures. Their existing protection-fault markers remain unchanged.

The separate `userspace-elf-probe` build runs the kernel-constructed static ELF
fixture instead of the resident userspace suite. It checks initialized data,
multi-page zeroed BSS, stack alignment, a returning unknown syscall (`-38`),
yield (`0`), preserved register/stack/data contents, and exit syscall `1` with
status `0`.
After one warmup and three measured runs, frame/node/process baselines must be
restored and the scheduler must be empty. Checkpoint A emits exactly
`memory_verification: PASS` and `userspace_elf: PASS`, each with the
`[FERBER_TEST]` prefix. These markers do not claim resident handle/channel
coverage or protection-fault containment. See
[the ELF loading contract](elf-loading.md).

Checkpoint B adds a test-only startup handoff to that ELF fixture. Its handle
table is empty at process birth; before scheduler admission, the kernel
explicitly installs one bootstrap channel endpoint with `READ | WRITE` rights.
The executable discovers its slot through the [versioned startup record](userspace-startup.md)
at initial SP, which replaces the original slot-0 test convention. The kernel
prequeues the challenge. Shared Rust fixture code receives
it with nonblocking syscall `9`, sends its reply with nonblocking syscall `8`,
and exits; the kernel verifies the reply and emits
`[FERBER_TEST] userspace_bootstrap_channel: PASS` only after cleanup checks.
There is no runnable service task, and the exchange does not use blocking
receive syscall `10`. Startup adds no syscall number and does not provide
init's service protocol.

The separate debug `cooperative-deadlock-probe` build runs two receivers without
a runnable producer. It must diagnose
`cooperative deadlock: live tasks with no runnable producer` with a kernel panic;
it must not emit `cooperative_scheduling`, `blocking_channel_receive`, or
`task_reclamation` success markers. Earlier memory and legacy syscall checks
may complete before the deliberate deadlock. Both architecture smoke suites
check the diagnostic rather than treating a timeout or an arbitrary panic as
success; the log parser rejects missing, duplicate, and truncated evidence.
