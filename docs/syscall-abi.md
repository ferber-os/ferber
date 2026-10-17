# Resident Syscall ABI

Ferber uses Linux-style argument registers with its own syscall numbers.
This ABI currently runs one resident task on one active CPU. It does not provide
Linux or musl compatibility, scheduling, or complete task/exit reclamation.

## Calling Convention

| Architecture | Number | Arguments 0–5 | Signed result |
| --- | --- | --- | --- |
| RISC-V 64 | `a7` | `a0`–`a5` | `a0` |
| x86-64 | `rax` | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` | `rax` |

Enter through `ecall` on RISC-V and `syscall` on x86-64. Unused arguments are
ignored. Returning calls preserve general registers other than the result and
the x86 `syscall` architectural clobbers (`rcx`, `r11`). x86 return flags are
sanitized. FP/vector state is outside the resident ABI.

## Operations

| Number | Name | Arguments | Successful result |
| --- | --- | --- | --- |
| `0` | Yield hint | None | `0`, immediately |
| `1` | Thread exit | Exit status | Does not return; marks the thread Dead and halts |
| `2` | Handle close | Handle | `0` |
| `3` | Handle duplicate | Handle | New handle number |
| `4` | Capability inspect | Handle, output address | `0` |
| `5` | Capability attenuate | Handle, rights mask | New handle number |
| `6` | Capability revoke | Handle | `0` |
| `7` | Channel create | Pair output address | `0`; writes two handles |
| `8` | Channel send | Endpoint handle, bytes address, byte count, handles address, handle count | `0` |
| `9` | Channel receive | Endpoint handle, bytes address, byte capacity, handles address, handle capacity, counts output address | `0` |

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

## Nonblocking Channel IPC

Create writes an eight-byte record containing two little-endian `u32` endpoint
handles. No input handle is required. Both endpoints are created and published
together. Their rights are `READ`, `WRITE`, `TRANSFER`, `DUPLICATE`, `INSPECT`,
and `MANAGE`; memory and execution rights are absent. Failure leaves the table
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

Receive requires `READ` and consumes the oldest message only when the supplied
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

Each incoming queue holds up to 64 messages. Full queues and empty open queues
return `-11`. Send to a closed peer returns `-32`; receive drains already queued
messages before returning `-32` for an empty closed queue. Endpoint closure is
observed after its last backing owner is released. Discarded messages release
node references without revoking shared aliases or descendants. Queue state
uses one lock per endpoint pair; discarded messages are destroyed after that
lock is released. Endpoint, message, and queue allocations are fallible.

These calls do not park threads, wait for readiness, or schedule another task.
Blocking receive and wait queues remain separate work.

## Errors and Validation Order

| Result | Meaning |
| --- | --- |
| `-9` | Invalid handle, including nonzero bits above bit 31 |
| `-11` | Channel queue is full, or empty with its peer still open |
| `-12` | Capability-node, endpoint, message, or queue allocation failed |
| `-13` | Required right is absent |
| `-14` | User buffer range is invalid or lacks the required user permissions |
| `-22` | Invalid rights mask, endpoint backing/type, repeated transfer slot, unsupported payload backing/type, or overlapping receive outputs |
| `-24` | Handle table is full |
| `-28` | Live/reserved node quota or node IDs exhausted |
| `-32` | Channel peer is closed and no queued message can be received |
| `-38` | Unknown syscall number |
| `-90` | Message exceeds limits or receive buffers are too small |
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
runtime dispatch.

`bash scripts/channel-tests.sh` runs host regressions against the channel core
for opposite-direction sends, deterministic allocation failures, full-queue
admission, failed-receive retry, and draining after peer closure. In-kernel
checks use real capability nodes to cover ownership and destructor reentrancy.

The resident test observer diagnoses a nonzero exit status or incomplete
coverage as a failed run, with a kernel panic rather than success markers.

A successful normal boot emits exactly one of each marker:

```text
[FERBER_TEST] privilege_switch_roundtrip: PASS
[FERBER_TEST] handle_syscalls: PASS
[FERBER_TEST] capability_mutation: PASS
[FERBER_TEST] channel_syscalls: PASS
```

Isolation, RX-write, and NX-execute probes remain separate builds without these
handle fixtures. Their existing protection-fault markers remain unchanged.
