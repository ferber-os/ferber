# Uranium-238 v0.2.1

*Userspace gains handle close, duplicate and inspect operations on both architectures. Authorization uses the calling process’s table; this release still runs one resident task whose exit halts the kernel.*

## Handle operations enforce the calling process's authority

Syscall `2` closes a handle. Syscall `3` requires `DUPLICATE` and returns a new
handle to the same capability node, with exactly the same rights and type.
Duplication consumes a handle slot without allocating another node. Closing
one alias leaves the others usable; revoking the shared node invalidates all
aliases. A revoked handle can still be closed.

Syscall `4` requires `INSPECT` and writes an eight-byte record: a little-endian
`u32` object type followed by a little-endian `u32` rights bitmask. It exposes
no pointers or internal node identifiers and does not dereference the object.
Unaligned destinations are supported. The existing user-copy helper validates
the entire output range before writing, so a failed copy leaves every output
byte unchanged.

The kernel resolves handles through trusted current-task state. Processes
still start with zero handles, and probe fixtures are delegated explicitly
after checking that empty-table contract. Handle arguments above `u32::MAX`
are rejected rather than truncated. Handle and rights validation precedes
output-memory or table-capacity checks.

The signed error ABI returns `-9` for invalid handles, `-13` for missing rights,
`-14` for invalid output memory, `-24` for a full handle table, and `-116` for
revoked capabilities. Unknown syscalls return `-38`. Yield (`0`) returns zero
independently of probe sequencing. Exit (`1`) marks the current thread Dead
and halts. These values and Linux-style argument registers do not provide
Linux or musl compatibility. The full interface is documented in
[the resident syscall ABI](../reference/syscall-abi.md).

## Resident probes check useful operations and rejected requests

Normal userspace probes exercise inspection, duplication, independent closure,
invalid and high-bit handles, insufficient rights, revoked capabilities,
table exhaustion and recovery, and final cleanup. Copy checks cover aligned,
unaligned, cross-page, read-only, unmapped, supervisor, null, and overflowing
output ranges. Kernel checks also cover shared-node revocation and more than
1,024 duplicate/close cycles without consuming capability-node allocations.

A normal boot must emit exactly one of each marker after the probe completes
successfully and leaves no handles:

```text
[WOLFRAM_TEST] privilege_switch_roundtrip: PASS
[WOLFRAM_TEST] handle_syscalls: PASS
```

Register, stack, and x86 flag checks remain in place. Separate supervisor-read,
RX-write, and NX-execute builds retain their protection-fault markers without
receiving handle fixtures. Smoke-log validation rejects missing or duplicate
markers, wrong scenarios, failures, and truncated output.

## Validation covers both architectures and a physical boot

Both the kernel and UEFI loader advance to `0.2.1`, with no dependency upgrades.
The implementation passed all 20 local QEMU boot cases: 10 RISC-V boots, nine
x86 UEFI directory boots, and one release x86 UEFI ISO boot. This includes
debug/release protection-fault probes on both architectures. The ISO smoke
kernel enables `qemu-debug-port` for log capture and is not byte-identical to
the ordinary release kernel.

Clippy passes with `-D warnings` for both kernel targets, their probe
configurations, and the UEFI loader. Debug/release freestanding x86 ELF checks,
formatting, shell syntax, and smoke-log parser self-tests also pass. These are
local results; they do not claim a hosted GitHub Actions run.

A release-profile ISO of this implementation booted on physical x86-64
hardware and reported a PASS followed by the expected halt. That run preceded
the version metadata bump and verifies the normal resident path. The dedicated
protection-fault builds were tested in QEMU, not on physical hardware.

## The runtime still has bootstrap limits

Execution remains one active CPU and one integer-only resident task. Timer
interrupts, preemptive/SMP scheduling, FP/vector user state, ELF loading, init,
musl, and complete task/object reclamation remain pending. The physical
allocator still manages at most 4096 MiB of RAM.

Handle numbers are reusable table slots. Close releases a slot without freeing
the capability node or object. Existing raw-node ownership and IPC message-drop
revocation behavior are unchanged; this release exposes metadata operations,
not channel syscalls. Concurrent unmapping and recoverable user-copy faults
remain future work.
