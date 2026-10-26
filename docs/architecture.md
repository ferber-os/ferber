# Ferber Architecture

Six locked decisions. These cannot be reversed.
Everything else in the kernel is built on top of them.

## 1. Microkernel

The kernel does: capability enforcement, scheduling, memory isolation, IPC, HAL.
Everything else (filesystems, drivers, networking) runs as unprivileged userspace
processes with explicitly granted capabilities.

No monolithic drivers. No hybrid. The capability model requires this.

HAL boundary: architecture-specific code lives in arch/ only.
The kernel core never touches hardware directly.

## 2. Capability Model

Every kernel resource is a typed object with handles.
Handles are unforgeable integers in per-process tables.
Every handle has a rights bitmask (DUPLICATE, TRANSFER, READ, WRITE, EXECUTE,
MAP, SIGNAL, WAIT, INSPECT, MANAGE).

Attenuation: you can only pass equal or fewer rights. Never more.
Revocation: capability nodes. Invalidate the node, all handles through it die instantly.
Rust layer: Handle<T, R> makes violations unrepresentable at compile time.

See docs/capability-model.md for full detail.

## 3. Process Model

Jobs -> Processes -> Threads.
No fork. No ambient inheritance.
Processes born with empty handle tables.
Spawn requires explicit capability handoff — pass exactly what the child needs.

Jobs form a tree. Killing a job kills everything in it recursively. No orphans.

Resident tasks cooperate through yield, blocking receive, and exit on one active
CPU with interrupts disabled. Existing architecture contexts preserve stackful
kernel continuations. A retained VMO-backed dispatcher stack runs on the
permanent kernel root; final task/process drops happen there outside locks after
switching away from the retired stack and address space. The last resident task
returns control to the kernel harness for reclamation checks. This does not add
preemption, FP/vector state, init, or task-creation syscalls.

The opt-in ELF checkpoint constructs a fresh process under the root Job and
loads a separately compiled static executable into VMO-backed user mappings.
The thread owns its entry PC and stack; the same dispatcher handles first entry,
yield, and exit. The process is born with an empty handle table. Explicit setup
installs the supplied bootstrap channel with `READ | WRITE` before admission.
A 32-byte versioned record at initial SP identifies the handle, and small entry
shims pass that record to shared `no_std` Rust code. The kernel
prequeues a challenge, and the fixture uses nonblocking channel syscalls to
reply for kernel verification. No service task is scheduled. The handle's
numerical slot is not an ABI constant. Image validation finishes
before task construction, and a failed construction is killed and detached
from the Job before returning. The independently built raw ELF arrives through
a bounded external boot payload: UEFI reads `BOOT.ELF`, while RISC-V uses FDT
initrd bounds. Its reserved source has a permanent supervisor RO/NX mapping
and boot-lifetime physical VMO ownership. Startup arguments, multi-file
initramfs delivery and service launching remain separate work.
The separate [minimal init](minimal-init.md) mode adds a kernel-owned bootstrap
exchange and exact terminal-wait validation without weakening ordinary
cooperative-deadlock detection. See
[the boot-payload contract](boot-payload.md),
[the ELF loading contract](elf-loading.md) and [startup ABI v1](userspace-startup.md).

## 4. IPC

Channels: async, bidirectional, bytes + handles, bounded queue, move semantics.
FastCall: sync, register-only, ~50-150 cycles, thread donation, interrupt/hot paths only.

Resident syscalls create channel pairs and send/receive without blocking on
both architectures. A pair has one state lock covering both incoming queues
and peer-open state, avoiding opposite-endpoint lock acquisition. Receivers
drain queued messages before observing peer closure.

Handle transfer is a move. The sender loses the selected table slots only when
the message is enqueued. Receive preflights output ranges and table space before
publishing any result. Failed operations preserve table entries, queued
messages, and user output. Discarding a message releases its references without
revoking aliases. Payload backing is limited to checked VMOs and the current leaf
Resource object; channels and task objects need a separate ownership-cycle
design before transfer. Blocking receive requires `READ | WAIT` and parks a
resident thread until message arrival, peer closure, or authority cancellation.
The dispatcher rechecks wait authority at cooperative boundaries. See the
[resident ABI](syscall-abi.md).

## 5. Memory

VMO-everything. No anonymous memory. No implicit backing.
Every virtual memory region backed by a named kernel object.
Rights enforced at page table level by MMU — hardware, not policy.

The physical bitmap allocator manages eligible firmware RAM across multiple
ranges. Page-sized metadata blocks grow with discovered capacity; the buddy
heap suballocates those physical pages. The heap's 128 MiB limit applies to
one allocation, independently of total managed RAM.

Both architectures run a high-half kernel with shared permanent supervisor
tables and empty lower halves in new process roots. Packed RAM aliases cover
eligible RAM without spending virtual space on physical holes or mapping
firmware reservations. Kernel text is RX, rodata RO/NX, and RAM aliases RW/NX;
the kernel image has no writable RAM alias. The packed alias windows are hard
capacity limits, including 2 MiB alignment padding: Sv39 provides 248 GiB,
while x86-64 is bounded by its window and CPU physical-address width. Exceeding
the limit fails boot. Allocator metadata placement depends on validating the
normalized physical map first. The current allocator lock is not safe for
same-CPU interrupt reentry; IRQ-context allocation and SMP use need an explicit
locking/publication design. See [the memory model](memory.md) for boot and
allocation limits.

## 6. Drivers

Userspace. Always. No exceptions.
MMIO: Physical VMO mapped into driver address space.
DMA: contiguous VMO, hardware-filled.
Interrupts: FastCall from kernel interrupt handler.
Three isolation tiers — all userspace.

## The Invariant

A process can only access resources it holds explicit capability handles for.
Enforced at: capability system (runtime), type system (compile time), MMU (hardware).
