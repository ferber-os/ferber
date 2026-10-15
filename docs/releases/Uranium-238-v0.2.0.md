# Uranium-238 v0.2.0

*Phase 3 M1 crosses the privilege boundary on RISC-V 64 and x86-64. Resident user probes return through syscalls, exit through the kernel, and separately prove that supervisor-only memory remains inaccessible.*

## User mode returns through a defined boundary

RISC-V enters U-mode with `sret`, switches to a retained kernel-stack VMO on
traps, and preserves user registers and nested supervisor state. x86-64 enters
Ring 3 through `iretq`; GDT/TSS state and `swapgs` select a trusted kernel stack
before the syscall entry touches user-controlled RSP. Both paths sanitize
initial user state and keep delegated interrupts disabled during M1.

The bootstrap ABI uses Linux-style registers with custom Wolfram numbers:
`0` yields immediately with result zero, `1` marks the bootstrap thread Dead
and halts, and the probe's unknown number returns `-38`. Each process starts
with an empty handle table. This ABI does not provide musl compatibility,
scheduling between user tasks, or complete exit reclamation.

## Supervisor mappings survive address-space activation

RISC-V maps the allocator-managed RAM bank supervisor-only, with RX kernel
text, RO/NX rodata, and RW/NX remaining RAM. x86-64 captures the loader's CR3
once, privately clones its lower-half table pages, and shares upper-half
kernel entries. Existing supervisor leaves retain their effective permissions
when new user mappings open ancestor entries. These bootstrap layouts remain
temporary until the permanent kernel mapping is introduced.

Allocated VMOs clear complete frames before exposure. Physical/MMIO VMOs
preserve existing memory. Kernel stacks require contiguous VMOs, and retained
owners keep active code, stacks, tables, and physical backing alive through
halt. Address-space teardown frees owned table frames rather than VMO frames.

## User copies validate every page before writing

Resident `copy_from_user` / `copy_to_user` helpers reject invalid ranges and
missing or insufficient page permissions. Whole-range preflight prevents
partial destination changes when a later page fails validation. Copies use
supervisor physical aliases and leave RISC-V SUM unchanged.

The unsafe caller contract requires stable mappings/backing, accessible RAM
aliases, and nonoverlapping kernel buffers. Concurrent unmapping, demand
paging, and recoverable copy faults remain future work.

## Boot evidence separates roundtrips from isolation faults

Normal boots require the exact marker
`[WOLFRAM_TEST] privilege_switch_roundtrip: PASS` after returning syscalls,
argument/register/stack checks, successful exit, and zero-handle validation.
The separate `user-isolation-probe` build requires
`[WOLFRAM_TEST] privilege_isolation: PASS` after validating a user protection
fault's origin, instruction, and address. The deliberate x86 invalid-opcode
build retains its expected exception/panic diagnostics.

The release preparation updates both the kernel and UEFI loader to `0.2.0`.
Clippy passes with `-D warnings` for both kernel targets, their probe
configurations, and the UEFI loader. The release UEFI build, debug/release
freestanding x86 ELF checks, shell syntax checks, and log-parser self-checks
also pass.

All 11 QEMU smoke cases pass. The six RISC-V cases cover normal debug/release
boots at 128 MiB/1 hart and 256 MiB/4 harts, plus debug/release isolation at
128 MiB/1 hart. The five x86 cases cover debug normal at 512 MiB, release
normal at 1024 MiB, release invalid-opcode at 512 MiB, and debug/release
isolation at 512 MiB. The Ubuntu CI workflow itself has not been run as part
of this local validation.

No physical-hardware run is claimed for this milestone. M1 runs one active
CPU with integer-only resident probes. Timer interrupts, preemptive/SMP
scheduling, FP/vector user state, init, ELF loading, and musl remain outside
this release's boundary. The earlier MSI B650M-A PRO WIFI boot evidence
belongs to Phase 1.
