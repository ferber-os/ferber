# Userspace startup ABI v1

The opt-in static ELF path enters a separately linked, integer-only `no_std`
Rust executable through this contract on RISC-V 64 and x86-64. It defines first
entry and bootstrap-handle discovery. The executable arrives through the
[external boot-payload handoff](boot-payload.md). Minimal init uses this same entry contract and a separate
[bootstrap protocol](minimal-init.md); service launching remains pending.

## Initial SP identifies a fixed startup record

The ELF entry receives a 16-byte-aligned stack pointer pointing to the record
below. The record occupies the top 32 bytes of the existing 16 KiB RW/NX user
stack. The unmapped guard below the stack and the full mapping extent remain
unchanged. No return address, arguments, environment, auxiliary vector or TLS
state is supplied. Other initial general registers are not startup arguments.

All fields use fixed-width little-endian encoding. `shared/startup.rs` is the
single source for encoding and validation; no Rust structure padding or kernel
pointers cross the boundary.

| Offset | Bytes | Field | v1 value |
| --- | --- | --- | --- |
| 0 | 8 | Magic | `FRSTART1` |
| 8 | 4 | Version | `1` |
| 12 | 4 | Record size | `32` |
| 16 | 4 | Bootstrap handle | Handle number, or zero when absent |
| 20 | 4 | Flags | Bit 0 indicates bootstrap presence; other bits zero |
| 24 | 8 | Reserved | Zero |

Handle zero is valid when the presence flag is set. Decoding requires exactly
32 accessible bytes, matching magic, version and size, known flags and zero
reserved bytes. An absent handle must encode as zero. The bootstrap program
requires a present handle and fails startup otherwise. Future layouts require
an explicitly supported version; oversized or unknown records are rejected.

The record is ordinary process-owned writable stack memory. It names an
existing capability and grants no authority by itself. The kernel never trusts
changes the program makes to this record. User-side validation operates on
accessible bytes; it does not provide recovery for an invalid or unmapped
startup pointer. Unexpected user faults remain fatal in the current harness.

## Delegation completes before the program becomes runnable

Processes are created with empty handle tables. The ELF setup explicitly
installs the supplied bootstrap endpoint before admission and records its
assigned slot. The current test uses a nonzero slot to reject a hardcoded-zero
assumption; that numerical slot is not part of the ABI. The program must read
the record.

The fixture endpoint has exactly `READ | WRITE`; minimal init explicitly
receives `READ | WRITE | WAIT` for blocking receive. The process owns its handle
and may close it; final process teardown also releases it. The kernel harness
retains only the opposite endpoint. The record owns no capability reference.
The loader can also construct an executable without a bootstrap endpoint,
encoding the absent form explicitly.

All segment, stack, metadata and delegation setup completes before scheduler
admission. A setup or admission failure kills and detaches the partial process
and releases its backing and capability ownership. Per-thread initial SP is
separate from the stack mapping extent so reclaiming the task still unmaps the
whole stack. Resident probes retain their existing stack-top entry convention.

## Small entry shims call shared Rust code

x86-64 copies entry RSP into RDI and calls the Rust entry function, giving it
the System V function-entry stack alignment. RISC-V passes entry SP in A0 and
maintains 16-byte stack alignment across the call. The bootstrap program uses
no allocator or TLS. Panic and unexpected return terminate unsuccessfully.

The RISC-V user executable uses `riscv64imac-unknown-none-elf` and its LP64 ABI;
the kernel continues to use `riscv64gc-unknown-none-elf`. This avoids compiling
user code for floating-point state the scheduler does not preserve. The build
script accepts the kernel target argument and selects the integer-only user
target explicitly. x86-64 uses the existing freestanding soft-float target.

## Verification includes Rust execution and cleanup

Shared Rust code validates the startup record and consumes the discovered
handle through ordinary channel syscalls. The kernel prequeues `ferber:1`;
the program receives it, sends `ready-v1`, closes the handle, yields and exits
zero. This remains a test exchange with no independently running service.

The ELF checks retain initialized data, multi-page BSS zeroing, writable memory,
unknown-syscall behavior and an assembly register/stack preservation check
around yield. Host tests cover malformed startup bytes, initialized stack
contents, initial-SP bounds and construction rollback. Boot tests require the
exact reply and restored frame, node, process and scheduler baselines before
the existing `userspace_elf` and `userspace_bootstrap_channel` PASS markers.
The external-delivery scenario additionally requires `boot_payload: PASS`.
These fixture markers do not establish init, service launching or recoverable
user faults. Minimal init has separate readiness and waiting markers.
