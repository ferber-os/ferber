# Wolfram Architecture

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

## 4. IPC

Channels: async, bidirectional, bytes + handles, bounded queue, move semantics.
FastCall: sync, register-only, ~50-150 cycles, thread donation, interrupt/hot paths only.

Handle transfer is a move. Sender loses the handle on send.

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
