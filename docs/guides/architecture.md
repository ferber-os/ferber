# Ferber keeps resource authority explicit

Ferber is a microkernel: its privileged core enforces capability operations,
address-space isolation, task execution and communication. Ferrum runs services
outside that boundary. A fault in a service must not grant it authority over
unrelated resources; each service receives explicit capabilities.

## A handle names an object and limits its use

A capability combines a resource reference with permitted operations. Programs
use integer handles to capabilities in their own tables. A number from another
process is not a usable reference. Every syscall validates the calling process's
handle, its rights and the actual object backing.

A process begins with an empty table. Its creator can install explicitly chosen
capabilities during construction. A filesystem service cannot create processes
merely because it can read executable bytes; task creation needs separate Job
authority. Explicit grants constrain confused-deputy mistakes, but services still
have to use their delegated authority correctly.

Duplication creates another reference to the same capability node. Derivation
creates a child with equal or fewer rights. Revoking a node invalidates that
node and its descendants; ancestors and sibling branches remain usable.
Closing a handle releases ownership. It does not automatically revoke aliases
or remove installed memory mappings. See the
[capability reference](../reference/capability-model.md).

## Init constructs services through the same boundaries as other programs

The boot environment supplies an initial executable and an opaque archive. The
kernel loads Ferrum init and delegates a bounded services Job plus read authority
over that archive. Init starts initfs, the read-only filesystem service, then
reads configuration and executable files through filesystem IPC.

Init prepares all configured service processes before starting them. Each child
receives its selected bootstrap channel and optional filesystem session. The
kernel does not resolve service paths. See [boot and services](boot-and-services.md)
for the complete sequence.

## Jobs own processes, and processes own threads

A Job is a container for processes and child Jobs. Killing a Job terminates its
subtree rather than reparenting surviving processes. Process construction is
staged: a controller owns a draft, mappings and delegated handles are installed,
a thread is prepared, then explicit start makes it runnable. Dropping the final
draft controller aborts construction; closing a controller after start does not
kill its child.

Threads currently cooperate through yield, blocking receive and channel wait
sets. One CPU executes user tasks with interrupts disabled. The kernel switches
to a retained dispatcher stack and permanent address space before reclaiming a
dead task's stacks and page tables. Preemption, SMP and FP/vector context are
outside the current execution contract. Unexpected user faults still follow a
fatal diagnostic path; general service fault recovery is not implemented.

## Memory access and authority have different lifetimes

A Virtual Memory Object (VMO) owns backing for user mappings. Mapping permissions
must be authorized by the capability used to install them. Page tables enforce
the installed access. Mappings retain owned backing even after the originating
handle is closed or revoked; revocation does not retroactively unmap memory.

Executable VMOs are sealed before executable mappings are published. Sealing
fails while writable mappings or other writable permits remain anywhere on the
object. The kernel checks both capability rights and object state.

Physical RAM may be sparse. The allocator validates usable ranges and subtracts
reservations before allocating frames. Both ports use permanent high-half kernel
mappings and packed RAM aliases, with architecture-specific capacity limits.
Details and unsafe caller obligations are in the [memory reference](../reference/memory.md).

## Channels carry messages and explicit transfers

Channels are paired, bounded message queues. Successful transfer moves selected
handles; failed operations preserve the documented source, queue and output
state. Closing a peer still permits already queued messages to be drained.
Messages currently carry eligible VMO and leaf Resource capabilities; ordinary
messages cannot carry channel endpoints or task objects.

Blocking receive waits on one channel. Wait-many observes up to 16 channel
endpoints without consuming a message. The scheduler checks whether blocked
components connect to an actual registered producer, distinguishing legitimate
idle from a disconnected wait cycle.

FastCall is a separate kernel primitive for synchronous register transfer and
thread donation. It is not exposed as a general userspace syscall. Performance
targets are not measured latency guarantees. See the [IPC reference](../reference/ipc.md).

## Architecture code supplies the machine boundary

RISC-V64 supplies Sv39 translation and U-mode entry; x86-64 supplies four-level
translation and Ring 3 entry. Architecture code implements trusted stack entry,
register preservation, return-state sanitization and translation/cache ordering.
Shared kernel contracts cover capabilities, VMOs, task construction and channels.
Typed Rust wrappers help express intended object and rights relationships; they
do not replace runtime syscall authorization or unsafe-code review.

The driver model places device services in userspace with explicit MMIO,
interrupt and DMA authority. General driver services and interrupt-driven
execution are still planned. AArch64, the shell and other priorities are tracked
on the [roadmap](../../ROADMAP.md).
