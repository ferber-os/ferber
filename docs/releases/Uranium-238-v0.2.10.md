# Uranium-238 v0.2.10

*Ferrum init now starts as an independently delivered Rust executable and remains alive while waiting for messages. The scheduler distinguishes that legitimate wait from cooperative deadlock.*

## Init owns a defined startup protocol

The release ISO now carries `userspace/init/` as `BOOT.ELF`. RISC-V receives the
same independently built program through its initrd path. Shared userspace
entry, syscall and linker support serves both init and the retained finite ELF
fixture. Kernel compilation still embeds no executable.

Startup ABI v1 and BootInfo v3 remain unchanged. The process is born with an
empty handle table, then receives one explicitly delegated `READ | WRITE | WAIT`
channel before admission. Its assigned handle comes from the startup record;
no slot number is an ABI. Init receives no job, namespace, device or process
creation authority.

A separate versioned 16-byte bootstrap protocol exchanges START and READY.
Both sides validate exact length, magic, version, opcode, reserved zero fields
and absence of transferred handles. Init then blocks on the same endpoint.
Malformed input, closure and unsupported subsequent messages cause failure.
Named ELF admission returns numeric process/thread identities while preserving
rollback on construction or admission failure. No new syscalls are introduced.

## A live init wait is distinct from cooperative deadlock

The kernel accepts terminal waiting only when init is the sole live task and
has sent exactly one valid READY reply. Its pending receive must match the
expected slot, capability node, channel pair, opposite endpoint side and actual
registered wait token. Rights, current generation, notification state and
cancellation errors are checked. Unexpected queued traffic prevents success.
An open peer or a task named init cannot waive these checks.

Validation runs on the permanent-root dispatcher stack outside locks. The
kernel retains its bootstrap peer and init's live resources through terminal
halt. Ordinary scheduler runs retain the exact diagnostic
`cooperative deadlock: live tasks with no runnable producer`. Unexpected init
exit is diagnosed after safe dispatcher reclamation rather than freeing an
executing stack. Existing cancellation and wait reconciliation remain active.

Every init boot first exercises the same executable through private
close-after-park runs: close the kernel peer, resume the blocked task, observe
its failure exit and reap it. A warmup and measured repetition restore frame,
capability-node and root-job process baselines before admitting the final live
instance. Final init resources intentionally remain owned.

## Verification covers successful waiting and rejected startup

Successful boots require `[FERBER_TEST]` markers for `memory_verification`,
`boot_payload`, `init_ready` and `init_waiting`. The last marker requires the
actual suspended receive; READY alone cannot satisfy it. Separate adversarial
executables exercise malformed readiness, closure, premature exit and waiting
on an unrelated channel without adding test commands to production init.
Strict log parsers reject contradictory, duplicate and truncated evidence.

Local validation passed 26 RISC-V cases and 23 x86 UEFI cases across main and focused
runs, plus the release init ISO. Final focused validation after
wait-registration strengthening passed RISC-V debug at 256 MiB/four harts and
the x86 release init ISO. All 61 host tests passed: 17 memory, 12 channel,
15 ELF parser, seven loader/startup, eight UEFI payload and two init protocol.
Both minimal-init kernel Clippy checks passed with `-D warnings`; default
RISC-V and x86 ELF checks, shell syntax and log-parser self-checks passed.

A physical x86-64 boot on the 32 GiB MSI workstation reached all four required
markers with the pre-bump `0.2.9` banner. The prerequisite close/resume/reclaim
checks therefore also completed. Hosted Actions results remain separate from
local validation.

The kernel and UEFI loader advance to `0.2.10` without dependency upgrades.
This remains a prerelease; `0.3.0` stays reserved. Execution is cooperative,
integer-only and single-CPU with interrupts masked. Terminal waiting has no
interrupt-driven event source or startup deadline. Init does not yet launch
services; userspace task creation, event-driven idle and a shell remain ahead.
