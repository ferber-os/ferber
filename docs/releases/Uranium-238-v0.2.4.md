# Uranium-238 v0.2.4

*Userspace can create paired channel endpoints, send bounded messages, and receive messages with explicitly transferred capabilities on RISC-V 64 and x86-64. Failed sends preserve sender handles; failed receives preserve the queued message and user outputs. The new channel syscall probe passed on physical x86-64 hardware with 32 GiB installed. One CPU still does all the work. Uranium has time.*

## Nonblocking channel syscalls cross the userspace boundary

Syscalls 7, 8, and 9 create endpoint pairs, send messages, and receive messages.
Both architectures use the same ABI, with little-endian handle and count fields.
Messages remain bounded to 64 KiB and 64 transferred handles; each endpoint
queue holds at most 64 messages. Calls return immediately when a queue is empty
or full.

Sending requires `WRITE` on the endpoint and `TRANSFER` on every payload
capability. Receiving requires `READ`. Payloads are accepted only when both
their capability type and concrete object backing identify a VMO or the current
leaf Resource object. Graph-owning objects remain excluded until their transfer
semantics prevent ownership cycles.

Create, send, and receive validate all affected user memory and handle-table
state before committing. A rejected send leaves source slots intact. A rejected
receive leaves the queued message, handle table, and output buffers intact,
including when output ranges alias the same physical memory. Queued messages
drain before peer closure is reported.

## Shared channel state removes opposite-send lock inversion

Each endpoint pair now uses one lock for both queues and endpoint liveness.
Sending no longer locks the peer endpoint, so opposite-direction sends do not
acquire endpoint locks in reverse order. Endpoint destruction detaches queued
messages while locked and drops their owned references after unlocking.

Discarding a message releases its capability references without revoking nodes,
aliases, or descendants. Explicit revocation remains controlled by `MANAGE`.
Channel and task capabilities are rejected as message payloads until the kernel
has a complete policy for graph ownership and cycles.

## Memory verification follows the detected map

The fixed 4 GiB threshold, dedicated high-frame capacity feature, and special
8 GiB smoke cases are removed. Verification now follows detected extent
boundaries and exercises ordinary VMO allocation, mapping, copies, unmapping,
and accounting regardless of RAM size. It checks representative boundaries;
it does not write across every physical frame.

The roadmap also places initramfs and bootstrap loading in Phase 3, an initial
userspace on-disk filesystem after block-device support in Phase 4, and native
NVMe plus the capability-native filesystem in Phase 6. The disk format and
write/crash-consistency contract remain future decisions.

## Validation includes emulators and physical x86-64

All 20 QEMU boot cases pass: 10 RISC-V, 9 x86 UEFI, and one release ISO boot.
The three channel host tests and nine memory host tests pass. Both kernel
targets, probe configurations, and the UEFI loader pass Clippy with
`-D warnings`. Smoke-log parser checks, shell syntax checks, and debug/release
x86 ELF validation pass.

The user booted the new release ISO on physical x86-64 hardware with 32 GiB
installed. Firmware memory discovery reported 31.46 GiB managed across 23
extents. Memory verification, privilege switching, handle syscalls, capability
mutation, and channel syscalls all passed before the kernel halted as designed.
No physical RISC-V run is claimed. Hosted CI is the remaining post-push check.

## Scheduling and general capability graphs remain unfinished

The kernel still runs one active CPU and one resident task. Blocking receive,
scheduler integration, general transfer of graph-owning objects, IRQ-safe
allocation, and SMP-safe locking remain future work. The memory verification
walks the detected map's boundaries and uses owned backing; it does not perform
an exhaustive destructive test of installed RAM.
