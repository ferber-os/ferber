# Static ELF executables

Ferber runs independently linked, integer-only static ELF executables on RISC-V64 and
x86-64. ELF describes which file bytes initialize each memory segment, the segment
permissions and the first instruction to execute.

The kernel uses the loader for init and the standalone ELF verification program. Ferrum
uses the shared parser when constructing filesystem and configured-service processes
through the [task ABI](task-creation.md). The same image restrictions apply to both
paths.

The standalone example is `make run USERSPACE_ELF_PROBE=1`; normal service startup uses
`make run INIT=1`. See [running Ferber](../guides/running.md) for setup.

## Accepted image format

`shared/elf.rs` parses ELF64 little-endian `ET_EXEC` images with System V
OSABI/version zero and the exact target machine (`EM_RISCV` or `EM_X86_64`). Limits are
4 MiB of file data, 32 program headers, 16 load segments, 16 MiB of page-rounded mapped
segments, and 256 section headers. Extended header numbering is unsupported.

All parsed table and load-payload ranges use checked bounds. Each nonempty `PT_LOAD`
requires `filesz <= memsz`, power-of-two alignment when greater than one, offset/address
congruence for both that alignment and 4 KiB pages, and a page-rounded range entirely
inside the target's lower user half above page zero. Rounded segment ranges must not
overlap. Permissions are exactly R-X, R--, or RW-; execute-only and writable-executable
mappings are rejected. The entry must be in file-backed executable bytes and, on RISC-V,
two-byte aligned.

Interpreter, dynamic-linking, TLS, executable-stack, relocation-section, and unknown
program-header requirements are rejected. Notes and PHDR descriptions need no runtime
processing; bounded RISC-V attributes are accepted. x86 ELF flags must be zero; RISC-V
flags may be `0`, `1`, or `5`. These flags do not prove instruction use: the fixture
must remain integer-only. Instruction validation, dynamic relocation, PIE user images,
and recoverable user faults are not supported by this image contract.

## Segment loading and process admission

`elf_load.rs` validates the image and excludes overlap with the fixed stack and its
guard before creating a fresh process under the root Job. Each load segment gets a
separate zeroed VMO. File bytes are copied at the in-page offset; BSS and unused page
tails stay zero. The process retains segment backing, and the thread retains its user
and kernel stack VMOs. Failed construction kills and detaches the partial process before
returning an error.

The user stack is 16 KiB, RW/NX, with an unmapped page below it. Initial SP points to a
32-byte, versioned startup record at its aligned top minus 32. No `argc`, `argv`,
environment, auxiliary vector or return address is installed. The thread's own entry PC
selects the ELF entry rather than the resident probe's fixed code address. The process
is born with an empty handle table; no authority is inherited.

The kernel loader may explicitly install a supplied bootstrap channel before admission
and encode its actual slot in the startup record. It can also encode an absent endpoint.
The standalone verification program receives READ | WRITE; init receives READ | WRITE |
WAIT. The record, not a slot convention, identifies the handle. See
[startup](userspace-startup.md).

## Building and delivering the standalone executable

`scripts/build-user-elf.sh` builds `userspace/elf-probe/` for the requested kernel
target and profile. RISC-V user code uses the integer-only
`riscv64imac-unknown-none-elf` target while the kernel remains `riscv64gc`. Install that
additional target with `rustup target add riscv64imac-unknown-none-elf --toolchain
nightly`. Small assembly entry shims call shared `no_std` Rust validation and protocol
code. Its linker script places `_start` at `0x400000` and emits separate R-X, R--, and
RW- segments. The kernel receives the independently built bytes through the
[boot-payload contract](boot-payload.md), not through `include_bytes!` or
`FERBER_USER_ELF`. Kernel builds do not require the fixture to exist. There is no
embedded fallback when the required payload is absent.

```bash
make run USERSPACE_ELF_PROBE=1
make debug USERSPACE_ELF_PROBE=1
make x86-iso USERSPACE_ELF_PROBE=1
```

These commands build and deliver the fixture separately. RISC-V uses QEMU's `-initrd`;
the x86 ISO carries `BOOT.ELF` beside the kernel on its FAT image. Without that option,
default builds retain the resident probes and need no payload. The raw ELF delivery
format does not provide an initramfs. The separate `init` mode receives an envelope and
loads [Ferrum init](init.md). Host parser and loader regressions validate image
restrictions and failed construction rollback. QEMU boots additionally exercise actual
mappings, entry, syscalls and cleanup. Their scope and exact markers are in
[verification](verification.md).

## Init admission

`make run INIT=1` and `make x86-iso INIT=1` build and deliver a dedicated init
executable. Admission supplies a process name and returns its kernel task identity for
bootstrap supervision. Segment permissions, guarded stack setup, startup ABI v1 and
construction rollback remain shared. Init receives `READ | WRITE | WAIT` and remains
alive after readiness; its retained resources must not be confused with the finite
fixture's reclamation checks. See [init](init.md) for the exact terminal-wait contract.
