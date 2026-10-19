# Static ELF loading: Checkpoint A

The opt-in `userspace-elf-probe` feature loads a separately compiled executable
on RISC-V 64 and x86-64. The kernel constructs its process and thread, then
uses the existing cooperative scheduler and syscall ABI. This checkpoint
does not start init or expose task creation to userspace.

## The accepted image is bounded and static

`ferber/src/kernel/elf.rs` parses ELF64 little-endian `ET_EXEC` images with
System V OSABI/version zero and the exact target machine (`EM_RISCV` or
`EM_X86_64`). Limits are 4 MiB of file data, 32 program headers, 16 load
segments, 16 MiB of page-rounded mapped segments, and 256 section headers.
Extended header numbering is unsupported.

All parsed table and load-payload ranges use checked bounds. Each nonempty `PT_LOAD`
requires `filesz <= memsz`, power-of-two alignment when greater than one,
offset/address congruence for both that alignment and 4 KiB pages, and a
page-rounded range entirely inside the target's lower user half above page
zero. Rounded segment ranges must not overlap. Permissions are exactly R-X,
R--, or RW-; execute-only and writable-executable mappings are rejected. The
entry must be in file-backed executable bytes and, on RISC-V, two-byte aligned.

Interpreter, dynamic-linking, TLS, executable-stack, relocation-section, and
unknown program-header requirements are rejected. Notes and PHDR descriptions
need no runtime processing; bounded RISC-V attributes are accepted. x86 ELF
flags must be zero; RISC-V flags may be `0`, `1`, or `5`. These flags do not
prove instruction use: the fixture must remain integer-only. Instruction
validation, dynamic relocation, PIE user images, and recoverable user faults
are outside this checkpoint.

## Construction completes before scheduler admission

`elf_load.rs` validates the image and excludes overlap with the fixed stack
and its guard before creating a fresh process under the root Job. Each load
segment gets a separate zeroed VMO. File bytes are copied at the in-page
offset; BSS and unused page tails stay zero. The process retains segment
backing, and the thread retains its user and kernel stack VMOs. Failed
construction kills and detaches the partial process before returning an error.

The user stack is 16 KiB, RW/NX, with an unmapped page below it. Initial SP
is its 16-byte-aligned top. No `argc`, `argv`, environment, auxiliary vector,
return address, or bootstrap channel is installed. The thread's own entry PC
selects the ELF entry rather than the resident probe's fixed code address.
The handle table starts and remains empty. No authority is inherited.

## The fixture is embedded independently linked data

`scripts/build-user-elf.sh` builds `userspace/elf-probe/` for the requested
target and profile. Its linker script places `_start` at `0x400000` and emits
separate R-X, R--, and RW- segments. The kernel embeds the resulting bytes
through `FERBER_USER_ELF`; the build script tracks changes to that file.
BootInfo version 2, UEFI handoff, and RISC-V FDT delivery are unchanged.
Embedding is a test mechanism pending a reserved boot payload for production.

For a RISC-V debug ELF kernel:

```bash
image=$(scripts/build-user-elf.sh riscv64gc-unknown-none-elf debug)
(cd ferber && FERBER_USER_ELF="$image" cargo build --features userspace-elf-probe)
```

`make x86-iso USERSPACE_ELF_PROBE=1` builds a release-profile ELF test ISO.
Without that option, the ISO and default builds retain the resident probes.
Run `bash scripts/elf-tests.sh` for parser regressions and
`bash scripts/elf-load-tests.sh` for loader regressions. The latter compiles
the production loader against fallible host resources: it checks data/zeroing,
permissions, guard collisions, and rollback after injected setup failures,
including partial mappings and admission failure. Its resource model does not
exercise real physical allocation, hardware page tables, or context switching.
Both architecture smoke suites include debug/release ELF scenarios;
`bash scripts/x86-iso-smoke.sh elf` checks the ELF ISO boot path. These are
verification entry points, not a claim that any particular run has passed.

The executable checks initialized read-only/writable data, multi-page BSS
zeroing, stack alignment, unknown-syscall return, cooperative yield with state
preservation, and exit. One warmup and three measured executions must restore
frame/node/process accounting and empty the scheduler before
`[FERBER_TEST] userspace_elf: PASS` is emitted. The ELF scenario also requires
`[FERBER_TEST] memory_verification: PASS`; it does not emit the resident suite's
syscall markers. Execution remains single-CPU, cooperative, integer-only, and
interrupt-disabled. A bad instruction or unexpected user fault still panics
the kernel. Bootstrap-channel startup is Checkpoint B; init and service
launching remain later work.
