# Uranium-238 v0.2.6

*Ferber can load a separately compiled static ELF into a fresh zero-handle process on RISC-V 64 and x86-64. The opt-in checkpoint validates the image, builds VMO-backed segments and a guarded stack, then runs it through the existing cooperative scheduler. A physical x86-64 boot reached the userspace ELF verification marker. Init and production image delivery remain ahead.*

## Static image validation happens before task construction

Checkpoint A accepts ELF64 little-endian `ET_EXEC` images for the exact target
architecture. It bounds input size, header counts, segment count, and mapped
memory before allocating process resources. Checked ranges reject truncated
tables, overflowing addresses, invalid alignment, page-zero mappings, overlapping
rounded segments, and entries outside file-backed executable bytes.

Load segments permit only R-X, R--, or RW- user mappings. Writable-executable
and execute-only segments are rejected. The parser rejects dynamic linking,
interpreters, PIE images, TLS, relocation sections, executable stacks, and
unsupported program-header requirements. Accepted ABI flags do not validate
the image's instructions; execution remains integer-only.

Limits are 4 MiB of file data, 32 program headers, 16 load segments, 16 MiB of
page-rounded segment memory, and 256 section headers. The stack and its guard
have a reserved range that load segments cannot overlap. The complete contract
is documented in [the ELF loading specification](../elf-loading.md).

## The process owns every loaded segment

Each segment receives a separate zeroed VMO. The loader copies file bytes at
their in-page offset and leaves BSS and page tails zero. Process ownership
retains those VMOs until teardown; page tables do not own leaf frames.

The kernel creates the process under the root Job and allocates a separate
16 KiB RW/NX user stack with an unmapped guard page below it. Initial SP is
the aligned stack top. No arguments, environment, auxiliary vector, return
address, or bootstrap channel are installed. The handle table starts and
remains empty for this fixture.

Construction completes before scheduler admission. Failed setup kills and
detaches the partial process. Process/thread allocation and ownership-slot
reservation use fallible paths. A thread now retains its own ELF entry PC;
the scheduler no longer assumes every user task enters the resident probe's
fixed code address. Existing yield and exit syscalls handle the loaded task;
no syscall numbers are added.

## An independently linked fixture checks execution and reclamation

The fixture is built separately for each architecture with its own linker
script, entry at `0x400000`, and separate text, read-only data, and writable
data segments. The kernel embeds the resulting file through `FERBER_USER_ELF`.
This is a test-delivery mechanism; BootInfo version 2 and firmware handoff
remain unchanged.

The program checks initialized data, multi-page zeroed BSS, writable data,
stack alignment, unknown-syscall return, cooperative yield with preserved
register/stack/data state, and successful exit. One warmup and three measured
executions must restore frame, capability-node, and root-Job process counts
and leave the scheduler empty before the harness prints:

```text
[FERBER_TEST] userspace_elf: PASS
```

The ELF scenario also requires `memory_verification: PASS`. It runs instead
of the resident userspace suite and does not establish handle/channel probe
coverage or recoverable user-fault isolation.

## The physical marker covers the opted-in fixture path

The maintainer supplied a physical x86-64 boot screenshot showing
`[FERBER_TEST] USERSPACE_ELF: PASS`. That marker follows successful fixture
loading, execution, syscall checks, and repeated reclamation checks. It does
not establish arbitrary executable compatibility, init startup, dynamic
linking, or physical RISC-V support.

All 40 local host tests passed: 15 parser, four loader, nine memory, and 12
channel tests. All 27 local QEMU/ISO boots passed: 13 RISC-V cases, 12 x86 UEFI
cases, and both normal and ELF release-ISO paths. The kernel targets and probe
configurations, including the independently compiled ELF fixtures, passed Clippy
with `-D warnings`; release UEFI Clippy and x86 ELF checks also passed. These
are local validation results; no hosted CI run is claimed.

Host regression suites cover malformed images and bounded parser behavior,
plus production loader construction against fallible model resources. Loader
checks include copied bytes, zeroing, permissions, guard collisions, partial
mapping failure, and scheduler-admission rollback. The model does not exercise
physical allocation, hardware page tables, or context switching. The physical
boot preceded the version metadata bump.

## The release ISO runs the ELF checkpoint

Enable the fixture with `userspace-elf-probe` and an absolute `FERBER_USER_ELF`
path. `make x86-iso USERSPACE_ELF_PROBE=1` produces an ELF-test ISO. The release
workflow also builds this feature and publishes it as `ferber.iso`, with no
emulator debug-port feature enabled. Ordinary builds and `make x86-iso` without
the option retain the resident probes. Both kernel and loader versions advance
to `0.2.6` without dependency upgrades.

Execution remains cooperative on one active CPU with interrupts disabled.
Init, bootstrap-channel delegation, startup arguments, reserved boot payloads,
userspace task-creation APIs, VMO and FastCall syscalls, preemption, SMP,
FP/vector state, concurrent unmapping, and recoverable user faults remain
future work. An unexpected user fault still follows the fatal kernel path.
