# Process and initial-thread construction

Task ABI v1 lets a caller with Job authority build an isolated child before it can
execute. The caller creates a draft, maps prepared VMOs, delegates capabilities,
prepares one initial thread and explicitly starts the process.

A process controller is the capability used to manage that construction. Closing its
final owner aborts an unstarted draft; closing it after successful start does not kill
the child.

The usual sequence is create → map → give → thread-create → start. The operations below
are shared by RISC-V64 and x86-64. They do not provide `fork`, `exec`, arbitrary thread
creation or a process-kill syscall.

## Operations require explicit authority

| Number | Operation | Arguments | Success |
| --- | --- | --- | --- |
| 17 | `sys_process_create` | Job handle, options (`0`) | New process-controller handle |
| 18 | `sys_process_map` | Controller, VMO handle, descriptor address | `0` |
| 19 | `sys_process_give` | Controller, source handle | Assigned child handle slot |
| 20 | `sys_thread_create` | Controller, entry PC, bootstrap slot or `u64::MAX`, options (`0`) | `0` |
| 21 | `sys_process_start` | Controller, options (`0`) | `0` |
| 22 | `sys_process_abort` | Controller, options (`0`) | `0` |

All unused argument registers must contain zero for these six operations. Handle zero is
valid. Handles above `u32::MAX` are rejected, except the explicit absent-bootstrap
sentinel in syscall 20.

Creation requires MANAGE through a live Job capability with concrete `JobAuthority`
backing. Its structural Job reference is weak: storing the capability in a descendant
process cannot make a Job/process ownership cycle. The resulting controller has MANAGE,
MAP, EXECUTE, DUPLICATE and INSPECT, but no TRANSFER. Its object type is Process; the
type tag alone is not sufficient for authorization or downcasting.

Mapping requires MANAGE and MAP on the controller. Start requires MANAGE and EXECUTE.
Give, initial-thread creation and abort require MANAGE. Operations other than abort also
revalidate the retained creating Job capability and its ancestry, then require an active
Job and open allocation budget. Retaining or duplicating a controller cannot bypass
revocation of the original Job authority. Abort remains available for cleanup after that
Job authority is revoked, provided the controller capability itself remains live. A
revoked controller can be closed.

## Mapping retains backing independently of handles

Syscall 18 reads exactly 48 bytes from validated caller memory. The descriptor contains
six little-endian `u64` fields:

| Byte offset | Field |
| --- | --- |
| 0 | Child virtual address |
| 8 | VMO byte offset |
| 16 | Length in bytes |
| 24 | Permissions: `1` RO, `3` RW/NX, `5` RX |
| 32 | Options, must be zero |
| 40 | Reserved, must be zero |

Base, offset and nonzero length must be 4 KiB aligned. Each mapping is bounded by the
existing 16 MiB user-VMO size limit, must fit the VMO's logical length, and must lie
entirely in the architecture's lower user range above the null page. There are at most
64 mapping records and 16,384 mapped VMO pages per child, in addition to the separately
prepared initial stack. Existing private-table limits also apply. Collisions reject the
entire operation without overwriting an existing mapping.

The caller needs MAP and the corresponding READ/WRITE/EXECUTE rights on a concrete
owned-RAM VMO. Physical/MMIO backing is rejected. RW mappings retain an object-wide
writable permit. RX mappings require sealed backing; RWX and execute-only permissions
are invalid. The fixed startup stack and its guard are reserved before the first child
mapping: no caller mapping may overlap `USER_STACK - 4096 .. USER_STACK + 16384`, where
`USER_STACK` is `0x2000000000`.

A successful child mapping retains backing even if the parent closes or revokes its VMO
capability. Backing charges remain with the original allocation owner; child mapping
metadata and private page tables are allocated under the target Job budget. Failed
mapping preparation releases provisional ownership and charges. There is no draft-unmap
operation in v1; choose another free range or abort the draft to discard an incorrect
completed mapping.

## Delegation moves authority rather than inheriting it

A newly constructed child has zero handles. Syscall 19 requires TRANSFER on the source
capability and atomically removes that source slot while installing the same capability
node into a free child slot. It returns the actual slot. The caller must derive an
attenuated capability first if the child should receive fewer rights. No implicit
duplication or attenuation occurs.

Only owned-RAM VMO capabilities, concrete leaf Resource capabilities and channel
endpoints can be given. Job, Process, Thread and other graph-owning backing is rejected.
Channel payloads retain their separate existing ownership restrictions; this direct
child handoff does not enable channel endpoints in queued messages. Failure leaves the
source handle and child table unchanged. Once delegation succeeds, a later construction
failure does not return that handle to the parent; abort or final controller release
closes it with the rest of the draft.

## One prepared thread becomes runnable exactly once

Syscall 20 prepares the sole initial thread. The entry must resolve to sealed RX user
memory installed through a recorded child mapping. RISC-V entry must be two-byte
aligned. The kernel allocates a 16 KiB kernel stack and a zero-filled 16 KiB RW/NX user
stack with an unmapped 4 KiB guard below it. It writes startup ABI v1 at the aligned
initial SP, with an explicitly selected bootstrap slot or no bootstrap handle.

A supplied bootstrap slot must name a live concrete Channel with READ, WRITE and WAIT.
No slot number is assumed. Initial-thread creation is transactional: failed stack,
thread or mapping allocation leaves the draft without a prepared thread and permits a
retry. A second successful preparation is rejected.

Start revalidates the Job authority, draft state, sole prepared thread, entry mapping
and bootstrap capability. Scheduler admission reserves all fallible storage before
publishing the ready task. Failure leaves a retryable draft; success changes the
controller to Started and publishes exactly one runnable thread without switching away
from the caller inside the syscall.

Draft controllers retain their process strongly. Duplicate/derived controller
capabilities share the same construction state. Closing the final controller owner
aborts an unstarted draft; explicit abort is also available and is idempotent. Aborting
detaches the process from its Job and releases its inactive address space before mapped
backing and stacks. Started controllers retain only weak process references. Closing
them or exiting the creator does not kill the started child, which remains owned by its
authorizing Job and scheduler until exit or Job termination. Final task reclamation runs
on the dispatcher stack.

The system admits at most 64 live Job-created processes, including drafts. Construction
failure and final process destruction release their process-count permits. The limit
does not reserve physical RAM: hierarchical Job quotas and physical allocation
exhaustion remain independent failure modes.

## Failures preserve construction state

| Error | Meaning |
| --- | --- |
| `-9` | Invalid source, controller, Job or bootstrap slot |
| `-12` | Physical or allocator exhaustion |
| `-13` | Required rights missing, mutable RX backing or sealed RW backing |
| `-14` | Invalid descriptor buffer, child range or unmapped entry |
| `-16` | Conflicting lifecycle operation, dead/closed Job or missing initial thread |
| `-17` | Mapping collision |
| `-22` | Invalid options/reserved/unused arguments, descriptor, type, backing, guard overlap or RISC-V entry alignment |
| `-24` | Parent or child handle table full |
| `-28` | Job, process-count, mapping, private-table or capability-node quota exhausted |
| `-90` | Mapping size exceeds the user-VMO bound |
| `-116` | Revoked controller, Job ancestry or delegated bootstrap capability |

Operations after successful start reject construction mutations with `-16`; abort also
rejects Started controllers. Aborted controllers reject all operations except repeated
abort. Validation order can select an earlier authority or input error before a later
lifecycle error. A recoverable failure does not promise that internal vector capacity
shrinks or that monotonic IDs are reused.

Host tests execute production task syscalls, containers, capability objects, VMOs and
user-copy validation with host page-table and scheduler substitutes. They cover
lifecycle aliases, explicit bootstrap records, mapping/authority rejections, target Job
charging, live-process limits, admission retry and forced allocation failures. They do
not demonstrate native context switching or actual page-table enforcement; the
architecture boot suites supply that evidence. Execution remains cooperative,
integer-only and single-CPU with interrupts masked. IRQ allocation, concurrent mapping
changes, SMP, multiple initial threads and independent user-fault recovery are outside
this contract.
