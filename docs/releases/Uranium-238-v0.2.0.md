# Uranium-238 v0.2.0

*User probes now cross into U-mode or Ring 3, return through kernel syscalls and exercise memory isolation on both architectures. The release also distinguishes successful roundtrips from deliberately faulting probes.*

## User mode returns through a defined boundary

RISC-V enters U-mode with `sret`, switches to a retained kernel-stack VMO on
traps, and preserves user registers and nested supervisor state. x86-64 enters
Ring 3 through `iretq`; GDT/TSS state and `swapgs` select a trusted kernel
stack before syscall entry touches user-controlled RSP. Both paths sanitize
initial user state and keep delegated interrupts disabled during M1.

The bootstrap ABI uses Linux-style registers with custom Wolfram numbers:
`0` yields immediately with result zero, `1` marks the bootstrap thread Dead
and halts, and an unknown syscall returns `-38`. The x86 probe also enters
syscalls with the direction flag set and verifies that kernel entry clears
unsafe flags while syscall return sanitizes user flags and preserves
arithmetic flags.

Each process starts with an empty handle table. This bootstrap ABI does not
provide musl compatibility, scheduling between user tasks, or complete exit
reclamation.

## Supervisor mappings survive address-space activation

RISC-V maps the allocator-managed RAM bank supervisor-only, with RX kernel
text, RO/NX rodata, and RW/NX remaining RAM. x86-64 captures the loader's CR3
once, privately clones its lower-half table pages, and shares upper-half
kernel entries. Existing supervisor leaves retain their effective permissions
when new user mappings open ancestor entries. These bootstrap layouts remain
temporary until the permanent kernel mapping is introduced.

The physical allocator currently manages up to 4096 MiB of RAM. Supporting
system memory beyond that limit remains future work.

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

## Fault probes check the permissions hardware enforces

The normal probe requires
`[WOLFRAM_TEST] privilege_switch_roundtrip: PASS` after returning syscalls,
argument/register/stack checks, successful exit, and zero-handle validation.
Three separate builds test user reads from supervisor memory, writes to RX
code, and instruction fetches from an NX page. Each requires its own PASS
marker only after validating the fault origin, instruction, address, and
architecture-specific access type. The deliberate x86 invalid-opcode build
retains its expected exception and panic diagnostics.

The release updates both the kernel and UEFI loader to 0.2.0.
Clippy passes with `-D warnings` for both kernel targets, all three permission
probe configurations, the x86 exception configuration, and the UEFI loader.
Release UEFI and freestanding x86 ELF checks, shell syntax checks, and
smoke-log parser checks also pass.

All 20 local QEMU boots pass: 10 RISC-V cases, nine x86 UEFI directory boots,
and one x86 UEFI CD-ROM ISO boot using the release packaging script. The
RISC-V cases cover debug/release normal boots at 128 MiB/one hart and 256
MiB/four harts, plus debug/release supervisor-read, RX-write, and NX-execute
fault probes at 128 MiB/one hart. The x86 directory boots cover debug/release
normal, isolation, RX-write, and NX-execute probes, plus the release
invalid-opcode case. The ISO boot reaches the normal userspace roundtrip
marker. Its test kernel enables `qemu-debug-port` for log capture and is not
byte-identical to the release kernel. The GitHub Actions workflow was updated
to run these checks; this local validation does not claim a hosted CI run.

A physical x86-64 boot has now reached the normal userspace syscall roundtrip
PASS marker. The machine reported 4096 MiB of managed RAM, and all displayed
allocator, memory, paging, capability, task, and IPC boot checks passed.
The screenshot does not show the separate RX-write or NX-execute probes.

M1 runs one active CPU with integer-only resident probes. Timer interrupts,
preemptive/SMP scheduling, FP/vector user state, `init`, ELF loading, and musl
remain outside this release's boundary.
