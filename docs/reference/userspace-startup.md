# Userspace startup ABI v1

Every independently linked Ferrum executable starts with a small record on its user
stack. That record identifies an explicitly delegated bootstrap channel, if one was
supplied. A program reads the assigned handle number instead of assuming a fixed slot.

The kernel uses this contract to start init; init uses it for newly constructed
children. The [boot envelope](boot-bundle.md) separately delivers init and archive
bytes. The startup record does not encode file paths, service configuration or launch
authority.

This reference defines first entry on RISC-V64 and x86-64. Bootstrap messages are a
separate [init protocol](init.md).

## Initial SP identifies a fixed startup record

The ELF entry receives a 16-byte-aligned stack pointer pointing to the record below. The
record occupies the top 32 bytes of the existing 16 KiB RW/NX user stack. A 4 KiB
unmapped guard lies below the stack; the stack mapping still spans the full 16 KiB. No
return address, arguments, environment, auxiliary vector or TLS state is supplied. Other
initial general registers are not startup arguments.

All fields use fixed-width little-endian encoding. `shared/startup.rs` is the single
source for encoding and validation; no Rust structure padding or kernel pointers cross
the boundary.

| Offset | Bytes | Field | v1 value |
| --- | --- | --- | --- |
| 0 | 8 | Magic | `FRSTART1` |
| 8 | 4 | Version | `1` |
| 12 | 4 | Record size | `32` |
| 16 | 4 | Bootstrap handle | Handle number, or zero when absent |
| 20 | 4 | Flags | Bit 0 indicates bootstrap presence; other bits zero |
| 24 | 8 | Reserved | Zero |

Handle zero is valid when the presence flag is set. Decoding requires exactly 32
accessible bytes, matching magic, version and size, known flags and zero reserved bytes.
An absent handle must encode as zero. The bootstrap program requires a present handle
and fails startup otherwise. Future layouts require an explicitly supported version;
oversized or unknown records are rejected.

The record is ordinary process-owned writable stack memory. It names an existing
capability and grants no authority by itself. The kernel never trusts changes the
program makes to this record. User-side validation operates on accessible bytes; it does
not provide recovery for an invalid or unmapped startup pointer. Unexpected user faults
remain fatal in the current harness.

## Delegation completes before the program becomes runnable

Processes are created with empty handle tables. The ELF setup explicitly installs the
supplied bootstrap endpoint before admission and records its assigned slot. Programs
must read the record; the numerical slot is not part of the ABI.

The fixture endpoint has exactly `READ | WRITE`; init explicitly receives `READ | WRITE
| WAIT` for blocking receive. The process owns its handle and may close it; final
process teardown also releases it. The kernel harness retains only the opposite
endpoint. The record owns no capability reference. The loader can also construct an
executable without a bootstrap endpoint, encoding the absent form explicitly.

All segment, stack, metadata and delegation setup completes before scheduler admission.
A setup or admission failure kills and detaches the partial process and releases its
backing and capability ownership. Per-thread initial SP is separate from the stack
mapping extent so reclaiming the task still unmaps the whole stack. Resident probes
retain their existing stack-top entry convention.

## Architecture entry conventions

x86-64 copies entry RSP into RDI and calls the Rust entry function, giving it the System
V function-entry stack alignment. RISC-V passes entry SP in A0 and maintains 16-byte
stack alignment across the call. The bootstrap program uses no allocator or TLS. Panic
and unexpected return terminate unsuccessfully.

The RISC-V user executable uses `riscv64imac-unknown-none-elf` and its LP64 ABI; the
kernel continues to use `riscv64gc-unknown-none-elf`. This avoids compiling user code
for floating-point state the scheduler does not preserve. The build script accepts the
kernel target argument and selects the integer-only user target explicitly. x86-64 uses
the existing freestanding soft-float target.

## Init bootstrap protocol v3

The initial stack record remains exactly startup ABI v1. Init discovers its bootstrap
endpoint through that record and receives a separate 32-byte protocol v3 START message.
That message uses magic `FRINIT03`, a little-endian `u16` version `3` at offset 8,
opcode `1` at offset 10, zero reserved bytes at 12–15, the exact initramfs archive
length as `u64` at offset 16, and zero reserved bytes at 24–31. START carries exactly
two capability handles: the bounded launch Job authority and an RO-mappable sealed
archive VMO with TRANSFER. READY uses opcode `2`, the launched-child count (filesystem
plus configured services, 1–9) and no attached handles. Unknown versions, opcodes,
nonzero reserved bytes and unexpected lengths or handle counts fail startup.

For initfs, service configuration and child readiness, see [init](init.md) and
[filesystem sessions](initramfs.md). For the standalone test executable and reclamation
checks, see [verification](verification.md).
