# System call ABI

A system call crosses from an unprivileged program into the kernel. Ferber uses
Linux-style argument registers with its own operation numbers and capability checks;
these calls do not provide Linux or musl compatibility.

Calls 0–23 cover cooperative execution, handles, channels, VMOs and staged task
construction on both supported architectures. Execution currently uses one active CPU
with interrupts masked and integer-only user state.

The calling convention and operation table below are the interface reference. For a
complete process-entry example, start with [userspace startup](userspace-startup.md).
Test scenarios and expected markers are documented under
[verification](verification.md).

## Calling Convention

| Architecture | Number | Arguments 0–5 | Signed result |
| --- | --- | --- | --- |
| RISC-V 64 | `a7` | `a0`–`a5` | `a0` |
| x86-64 | `rax` | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` | `rax` |

Enter through `ecall` on RISC-V and `syscall` on x86-64. Unused arguments are ignored
for calls 0–16. Task-construction calls 17–22 and wait-many call 23 require unused
arguments to be zero. Returning calls preserve general registers other than the result
and the x86 `syscall` architectural clobbers (`rcx`, `r11`). x86 return flags are
sanitized. FP/vector state is outside the resident ABI. x86-64 clears `CR4.FSGSBASE` and
`CR4.PKE`, and sets `CR0.TS` before user execution. This prevents unsaved FS/GS bases
and PKRU state from passing between tasks, and traps FP/SIMD instructions. Unsupported
instructions follow the resident harness's fatal exception path.

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
| `11` | VMO create | Size, options (0, 1 or 3; MAP/EXECUTE opt-in) | New handle number |
| `12` | VMO read | Handle, offset, destination, length | `0` |
| `13` | VMO write | Handle, offset, source, length | `0` |
| `14` | VMO map | Handle, offset, address, length, permissions, options (`0`) | `0` |
| `15` | VMO unmap | Address, length; exact user record | `0` |
| `16` | VMO seal | Handle, options (`0`); requires MANAGE and EXECUTE | `0` |
| `17` | Process create | Job handle, options (`0`) | Process-controller handle |
| `18` | Process map | Controller, VMO handle, 48-byte descriptor address | `0` |
| `19` | Process give | Controller, source handle | Assigned child handle slot; source is moved |
| `20` | Initial thread create | Controller, entry, bootstrap slot or `u64::MAX`, options (`0`) | `0` |
| `21` | Process start | Controller, options (`0`) | `0` |
| `22` | Process abort | Controller, options (`0`) | `0` |
| `23` | Channel wait-many | Handle-array address, count (1–16), four zero arguments | Lowest ready index; consumes no message |

The [task-creation contract](task-creation.md) defines descriptor layout, explicit Job
authority, draft lifecycle, target allocation ownership and atomic publication. One
initial thread is supported; this is not a general multithread-creation ABI.

Every handle is resolved through the trusted calling process's table. Processes are born
with zero handles; delegation is explicit. Handle zero is valid. Handle arguments
greater than `u32::MAX` are rejected rather than truncated.

Close releases one table entry without requiring additional rights. A revoked handle can
be closed. Close releases its owned node reference without revoking aliases. Nodes and
their backing are reclaimed when their final owners are gone; other handles, typed
handles, messages, or descendants may retain them. Closing an already empty slot is an
error.

Duplicate requires `DUPLICATE` and inserts an alias to the same node with the same
rights and type. It does not consume a capability-node allocation. Closing one alias
leaves the others usable; revoking the shared node invalidates all aliases and
descendants. The table currently has 256 slots. Vacant numbers can be reused, so a
previously closed number can later refer to another capability.

Inspect requires `INSPECT`. It copies this fixed eight-byte record to the supplied
writable user address, which need not be aligned:

| Byte offset | Field | Encoding |
| --- | --- | --- |
| `0` | Object type | Little-endian `u32` |
| `4` | Rights bitmask | Little-endian `u32` |

Object types are Process `0`, Thread `1`, VMO `2`, Channel `3`, Job `4`, and Resource
`5`. Rights use the bit positions in the [capability model](capability-model.md#rights).
No pointers, internal node IDs, parent data, or names are exposed, and the object
pointer is not dereferenced.

The copy checks the entire range before writing. Null, overflowing, supervisor,
unmapped, or nonwritable destinations fail without changing any output byte.
Mappings/backing remain stable under the current resident-task constraints; concurrent
unmapping and recoverable copy faults remain future work.

## VMO Byte Operations

Create returns fresh zeroed owned RAM, bounded to 16 MiB and charged for its object
lifetime. Read/write require READ/WRITE respectively, copy exactly at most 64 KiB and
preflight complete ranges before staged copying. Physical VMOs are excluded. See [the
VMO byte ABI](vmo-abi.md) for exact rights, errors, zero-length behavior, the global
budget and aliasing guarantees. [Mapping and unmapping](vmo-mapping.md) require explicit
MAP authority and retain backing independently of handles. RO/RW-NX mappings remain
supported; RX additionally requires explicit EXECUTE authority and [irreversible object
sealing](vmo-sealing.md). Seal fails busy while any writable mapping survives; sealed
backing denies writes and RW maps.

## Capability Mutation and Ownership

Attenuate requires `DUPLICATE` and creates a distinct child node with the same object
and an equal or smaller rights mask. The source remains present. Equal rights still
create a child; a zero-rights child is allowed. Mask bits above bit 31 or outside the
defined rights set return `-22`; requesting defined rights absent from the source
returns `-13`.

Revoke requires `MANAGE`. It invalidates the selected node, all its aliases, and all
descendants. Ancestors and siblings remain valid. A repeated revoke returns `-116`;
revoked handles can still be closed. Revocation denies future capability operations. It
does not remove existing VMO mappings or reclaim tasks.

Tables, typed handles, and messages own `Arc<CapNode>` references. Each node owns its
parent and object. A live descendant therefore retains its ancestry and backing even
after all ancestor handles close. Validity checks walk ancestors with Acquire loads;
revocation uses one Release store. Dropping an unshared ancestry chain is iterative
rather than recursive.

The node factory allows 1,024 live or reserved nodes globally, including roots and
descendants. Duplicate aliases do not consume this quota. Child allocation uses fallible
heap storage and checked `u64` IDs; failed allocations release reserved quota. Closing
handles can make quota available again. ID exhaustion is permanent, and IDs are not
exposed through inspection.

## Channel IPC

Create writes an eight-byte record containing two little-endian `u32` endpoint handles.
No input handle is required. Both endpoints are created and published together. Their
rights are `READ`, `WRITE`, `WAIT`, `TRANSFER`, `DUPLICATE`, `INSPECT`, and `MANAGE`;
memory and execution rights are absent. Failure leaves the table and output unchanged.

Send copies at most 65,536 bytes and 64 little-endian `u32` handle numbers into one
message. Empty messages are valid. The endpoint requires `WRITE`; each payload handle
must be valid and carry `TRANSFER`. On success the selected source slots are removed and
their owned node references move into the peer's queue. Repeated source slot numbers
return `-22`. Distinct slots that alias one node are allowed. Payloads are limited to
VMOs and the current leaf Resource object, with both their type tag and actual backing
checked. Channel, Process, Job, Thread, and other backing return `-22`, including
aliases and descendants. Graph-owning payloads need a lifetime policy before queues can
retain them safely.

Nonblocking receive (`9`) requires `READ` and consumes the oldest message only when the
supplied buffers and handle table have room. Its eight-byte counts record contains the
received byte count followed by handle count, both little-endian `u32` values. The
handles buffer receives little-endian `u32` destination slots; unused output capacity is
untouched. Short buffers return `-90` and preserve the message for retry. A full
receiver table returns `-24` with the same preservation guarantee. Revoked in-flight
nodes remain closeable but provide no usable authority.

All nonempty user ranges are checked for overflow, effective user permissions, and
complete mapping before any output changes. Null addresses are allowed for zero-length
buffers. Receive rejects overlapping nonempty byte, handle, and counts output ranges
with `-22`, including distinct virtual ranges aliasing the same physical bytes.
Successful receipt publishes bytes, new handle slots, and counts together under the
resident stable-mapping contract. Every failed receive leaves user outputs, the queue,
and the table unchanged. Every failed send preserves the sender's exact source slots and
queue contents.

Each incoming queue holds up to 64 messages. Send to a full queue and nonblocking
receive from an empty open queue return `-11`. Send to a closed peer returns `-32`;
receive drains already queued messages before returning `-32` for an empty closed queue.
Endpoint closure is observed after its last backing owner is released. Discarded
messages release node references without revoking shared aliases or descendants. Queue
state uses one lock per endpoint pair; discarded messages are destroyed after that lock
is released. Endpoint, message, and queue allocations are fallible.

Blocking receive (`10`) requires both `READ` and `WAIT`, including when a message is
already queued. It uses the same capacities, buffer validation, publication, and retry
rules as `9`. An empty open queue parks the calling thread. Message arrival or final
peer closure makes the waiter runnable; readiness is rechecked before consuming a
message, so another receiver may win and the caller may park again. There is no timeout
and no blocking send.

A suspended receive retains its original capability identity. Closing its source slot
cancels it with `-9`, even if a readiness notification already occurred; reusing the
numeric slot cannot retarget the wait. Revocation of that node or an ancestor cancels it
with `-116`. The dispatcher scans waits at cooperative boundaries and revalidates
authority before resumed delivery. Cancellation does not change user outputs or consume
a message. Revocation denies authority; it does not itself close the endpoint. Endpoint
closure still requires release of its final backing owner.

## Cooperative Scheduling and Exit

Yield places a running thread behind other ready threads and returns `0` when it
resumes. With no other runnable task, it resumes itself. Kernel continuations use the
architecture's existing callee-saved `CpuContext`; each task retains its trap/syscall
frame on its VMO-backed kernel stack. Address-space and trusted kernel-entry stack state
are selected before entering that task's userspace.

Exit marks the calling thread Dead and never resumes its continuation. The
permanent-root dispatcher runs on a retained VMO-backed stack, so the exited thread's
active stack and tables are not freed while in use. Final thread and last-thread process
resources are detached and destroyed outside scheduler, process, handle-table, and
channel locks. This includes handles whose destruction closes endpoints or discards
queued messages. Surviving tasks continue running. When all resident tasks exit, the
scheduler returns to the kernel harness, which checks reclamation before halting. Live
tasks all blocked without a runnable wake source are diagnosed as deadlock, not
successful completion. The separate [init session](init.md) accepts only its exact,
validated bootstrap exchange and rooted multi-service wait graph as terminal waiting.
Registered sources have checked generations and weak endpoint identity. An open peer
alone provides no exemption or interrupt-driven wakeup guarantee.

Scheduling remains cooperative, single-CPU, and integer-only. Interrupt-driven wakeup,
preemption, SMP, concurrent unmapping, and FP/vector state remain outside this ABI.
Userspace-created tasks use the same scheduler and exit reclamation machinery. The
separately compiled ELF fixture uses this same syscall table; ELF loading adds no
syscall numbers.

## Errors and Validation Order

| Result | Meaning |
| --- | --- |
| `-9` | Invalid handle, including nonzero bits above bit 31, or suspended receive cancelled by source-slot close |
| `-11` | Channel queue is full, or empty with its peer still open |
| `-12` | Capability-node, endpoint, message, queue, VMO frame/metadata/object, page-table, or staging allocation failed |
| `-13` | Required right is absent, write/RW map targets sealed backing, or RX map targets mutable backing |
| `-14` | User buffer range is invalid or lacks the required user permissions |
| `-16` | VMO seal is blocked by a live writable-mapping permit, or task construction conflicts with lifecycle/Job state |
| `-22` | Invalid rights mask, options, permissions, alignment, logical VMO range or backing/type; repeated transfer slot; unsupported payload; or overlapping output bytes |
| `-17` | Mapping destination is already occupied |
| `-24` | Handle table is full |
| `-28` | Hierarchical Job allocation budget (including closed ancestry), user-VMO backing, mapping/alias/table limit, node quota, node IDs, live processes, or wait generations exhausted |
| `-32` | Channel peer is closed and no queued message can be received |
| `-38` | Unknown syscall number |
| `-90` | VMO/mapping size, byte-copy or message limit exceeded, or receive buffers are too small |
| `-116` | Capability node is revoked |

For duplicate, inspect, attenuate, and revoke, validate the handle, then node validity
(including ancestors), then the required right. Attenuation then checks mask encoding,
the rights subset, table capacity, and node allocation, in that order; it rechecks
source validity before publishing the child. Duplication checks capacity after source
rights; inspection checks output memory after source rights. Rejected calls leave the
table unchanged. Inspection-copy failures also leave the output unchanged. These error
values do not imply Linux ABI compatibility.

For channel send and receive, validate the endpoint handle, node validity, required
right, and actual endpoint backing before other arguments. Send then checks size limits,
reserves byte storage, copies bytes and handle IDs, then validates each payload in
order. It reserves handle and queue storage and checks peer/queue admission before
removing any source slot. Receive checks peer/queue state, head-message capacities,
table space, and all output ranges before copying or dequeuing. Create validates its
output range and two free slots before allocation, and rechecks them before publication.

## Waiting on several channels

`sys_chan_wait_many(handles_ptr, count, 0, 0, 0, 0)` copies an array of little-endian
`u32` handle slots, with `count` from 1 through 16. Every handle must name a live
channel with READ and WAIT rights. Duplicate endpoint identities, including aliases, are
rejected. The result is the lowest ready input index; a queued message or closed peer is
ready. No message is consumed and no timeout is supplied. Existing receive calls
distinguish queued data from peer closure.

All entries are authorized before any ready result. Invalid pointers return `-14`,
invalid slots `-9`, revoked nodes `-116`, missing rights `-13`, and invalid counts,
duplicate endpoints or nonzero reserved arguments `-22`. Closing or revoking any watched
slot cancels the entire suspended wait, including slot reuse. Every registration shares
a generation and is removed on return, cancellation, setup failure or task teardown.
Rooted-idle analysis includes every member; waiting on a filesystem session creates no
implicit external wake source.
