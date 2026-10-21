# Uranium-238 v0.2.8

*Startup ABI v1 places a 32-byte versioned little-endian record at the ELF fixture's initial stack pointer. Shared `no_std` Rust code decodes it, discovers the explicitly delegated bootstrap channel in a nonzero handle slot, receives the prequeued challenge, sends the reply, closes the handle, yields, and exits. The existing protocol order remains unchanged on both architectures. Uranium-238's half-life is longer than this protocol's lifetime.*

## The startup record describes a test process without granting authority

Processes are still created with empty handle tables. The ELF loader can
explicitly install a single `READ | WRITE` bootstrap channel before scheduler
admission and writes the assigned handle number into a fixed 32-byte record at
initial SP. The shared ABI uses fixed-width little-endian fields, a magic,
version, size, presence flag, and zeroed reserved bytes. It rejects malformed
lengths, versions, flags, reserved values, and inconsistent absent handles.
Handle zero remains representable; the test deliberately uses slot 7 to prove
that startup code discovers the handle from the record.

The process keeps its full stack mapping and guard page. Only initial SP points
at the record near the top of the 16 KiB stack. Explicit stack-pointer bounds
and alignment are validated independently of the stack mapping extent, so
teardown still releases the full stack. Loader setup and scheduler-admission
failures unwind the process, mappings, stack, and delegated capability.

## Shared Rust preserves the Checkpoint B contract

The ELF fixture is now a shared integer-only `no_std` Rust program with small
architecture entry and syscall shims. RISC-V userspace builds for
`riscv64imac-unknown-none-elf` and LP64 because the scheduler does not preserve
user FP state; the kernel remains `riscv64gc-unknown-none-elf`. x86-64 retains
its freestanding target. The shared code validates the startup record and
channel handle, then runs the existing sequence: unknown syscall, receive,
send, close, yield, and exit. Kernel-side response verification and cleanup
checks remain in place. Initialized data, BSS, register, and stack preservation
checks remain part of the fixture.

This does not define init's production ABI, launch services, or add `argc`,
`argv`, environment, auxiliary-vector, or task-creation support. The ELF image
is still embedded for opt-in test builds. Startup pointer failure paths remain
fatal under the current user-fault policy; host parser tests do not execute the
kernel panic path or emulate a malformed boot record fault.

## Local validation covers both architectures and the ELF ISO

Host suites pass 43 tests: 15 ELF parser, seven loader/startup, nine memory,
and 12 channel tests. A separate red-team mutation run checks 65,536 startup
record mutations. QEMU passes all 27 boots: 13 RISC-V cases, 12 x86 UEFI
cases, and normal plus ELF release-ISO boots. Kernel base and ELF-probe Clippy
checks pass with `-D warnings` on both targets; both separately compiled user
fixtures pass debug and release warning-free builds. Script self-tests and
syntax checks pass. Hosted GitHub Actions is not claimed as locally run.

A physical x86-64 boot on the MSI workstation showed
`memory_verification`, `userspace_elf`, and `userspace_bootstrap_channel` pass
markers on the ELF path. The boot banner reported `0.2.7`, before this version
bump. No physical RISC-V run is claimed.

The kernel and UEFI loader advance to `0.2.8` without dependency upgrades.
This remains a prerelease. Version `0.3.0` is reserved for the next milestone.
