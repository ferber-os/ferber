# Static ELF loading: Checkpoints A and B

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
points to a 32-byte, versioned startup record at its aligned top minus 32.
No `argc`, `argv`, environment, auxiliary vector or return address is installed.
The thread's own entry PC selects the ELF
entry rather than the resident probe's fixed code address. The process is born
with an empty handle table; no authority is inherited.

Checkpoint A established zero-handle construction. The current ELF path
explicitly installs the supplied bootstrap channel with `READ | WRITE` rights
after process construction and before scheduler admission, then records the
assigned slot in the startup record. A missing endpoint is encoded explicitly.
Slot `0` is no longer assumed; the test uses a nonzero slot. The kernel prequeues a fixed
challenge. The ELF fixture uses the existing nonblocking receive syscall (`9`)
to read it and nonblocking send syscall (`8`) to return a reply, which the
kernel verifies after the fixture exits. There is no runnable service task and
no blocking receive in this exchange. See [startup ABI v1](userspace-startup.md)
for the entry record, handle ownership and architecture calling conventions.
The exchange remains a test protocol.

## The fixture arrives as an external boot payload

`scripts/build-user-elf.sh` builds `userspace/elf-probe/` for the requested
kernel target and profile. RISC-V user code uses the integer-only
`riscv64imac-unknown-none-elf` target while the kernel remains `riscv64gc`.
Install that additional target with
`rustup target add riscv64imac-unknown-none-elf --toolchain nightly`.
Small assembly entry shims call shared `no_std` Rust validation and protocol
code. Its linker script places `_start` at `0x400000` and emits
separate R-X, R--, and RW- segments. The kernel receives the independently
built bytes through the [boot-payload contract](boot-payload.md), not through
`include_bytes!` or `FERBER_USER_ELF`. Kernel builds do not require the fixture
to exist. There is no embedded fallback when the required payload is absent.

```bash
make run USERSPACE_ELF_PROBE=1
make debug USERSPACE_ELF_PROBE=1
make x86-iso USERSPACE_ELF_PROBE=1
```

These commands build and deliver the fixture separately. RISC-V uses QEMU's
`-initrd`; the x86 ISO carries `BOOT.ELF` beside the kernel on its FAT image.
Without that option, default builds retain the resident probes and need no
payload. The raw ELF delivery format does not provide an initramfs or init.
Run `bash scripts/elf-tests.sh` for parser regressions and
`bash scripts/elf-load-tests.sh` for loader regressions. The latter compiles
the production loader against fallible host resources: it checks data/zeroing,
permissions, guard collisions, and rollback after injected setup failures,
including partial mappings and admission failure. Its resource model does not
exercise real physical allocation, hardware page tables, or context switching.
Both architecture smoke suites include debug/release ELF scenarios;
`bash scripts/x86-iso-smoke.sh elf` checks the ELF ISO boot path. These are
verification entry points, not a claim that any particular run has passed.

The Rust executable checks startup-record decoding and handle discovery,
initialized read-only/writable data, multi-page BSS
zeroing, stack alignment, unknown-syscall return, cooperative yield with state
preservation, and exit. One warmup and three measured executions must restore
frame/node/process accounting and empty the scheduler before
`[FERBER_TEST] userspace_elf: PASS` is emitted. The ELF scenario also requires
`[FERBER_TEST] memory_verification: PASS`,
`[FERBER_TEST] userspace_bootstrap_channel: PASS`, and
`[FERBER_TEST] boot_payload: PASS`. The last marker follows execution and
reclamation using the external source; delivery alone cannot satisfy it.
It does not emit the resident suite's
syscall markers. Execution remains single-CPU, cooperative, integer-only, and
interrupt-disabled. A bad instruction or unexpected user fault still panics
the kernel. Bootstrap-channel startup is Checkpoint B; init and service
launching remain later work. Checkpoint B must not be read as init or as a
general userspace service protocol.
