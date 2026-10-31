# Uranium-238 v0.2.2

*Userspace gains capability attenuation and subtree revocation. Capability-node ownership keeps ancestors and object backing alive until their final reference disappears.*

## Capability mutation preserves the authority boundary

Syscall `5` requires `DUPLICATE` and creates a distinct child capability with
equal or fewer rights. Equal and zero-rights masks are permitted. The source
handle remains present. Undefined or oversized masks return `-22`; requesting
defined rights absent from the source returns `-13`.

Syscall `6` requires `MANAGE` and revokes the selected node, its aliases, and
all descendants. Validity checks walk owned parent links. Ancestors and
siblings remain valid. Later operations through revoked nodes return `-116`,
including a repeated revoke; revoked handles can still be closed. Revocation
does not remove existing VMO mappings or perform task teardown.

Both architectures expose the same numbers, rights, and errors. Validation
checks the handle, ancestry validity, and required right before considering
the rights mask, table capacity, or allocation. Processes still start with
zero handles and receive probe fixtures through explicit delegation.

## Ownership makes node capacity recoverable

Handle tables, typed handles, and IPC messages own `Arc<CapNode>` references.
Each node owns its parent and object backing. A descendant keeps its ancestry
alive after ancestor handles close. Final owners reclaim nodes and backing;
unshared ancestry chains are dropped iteratively.

The factory bounds live and reserved nodes globally at 1,024. Roots and derived
children consume quota; duplicate aliases do not. Fallible allocation and
checked IDs return `-12` for heap failure or `-28` for exhausted quota or IDs.
Failed allocations release reservations. The existing `-9`, `-13`, `-14`,
`-24`, `-38`, and `-116` error meanings remain unchanged. Handle closure can
release quota, while node IDs are never reused.

Rejected IPC sends return the error and owned message with its carried nodes
already revoked. Callers release the endpoint lock before dropping that
message. This lets final object destruction acquire kernel locks safely.
Failed receipt into a full table returns the node reference to the caller.
Undelivered-message revocation still invalidates aliases and descendants.
These are trusted-kernel IPC paths; no channel syscalls are added.

## Resident probes check mutation and cleanup on both architectures

User probes exercise attenuation, inspection, child revocation, ancestor and
sibling survival, repeated revocation, malformed masks, denied rights, and
final handle cleanup. Kernel checks cover retained ancestry, more than 1,024
derive/close cycles, quota exhaustion and recovery, allocation-failure rollback,
deep-chain teardown, and destruction outside process/table/endpoint locks.

Every successful normal boot requires exactly one of each marker:

```text
[WOLFRAM_TEST] privilege_switch_roundtrip: PASS
[WOLFRAM_TEST] handle_syscalls: PASS
[WOLFRAM_TEST] capability_mutation: PASS
```

Supervisor-read, RX-write, and NX-execute probes remain separate builds.
Register, stack, and x86 flag checks remain in place. The code VMO now follows
the resident probe's actual length rather than assuming a single page.

## Local validation includes the release ISO and physical hardware

The kernel and UEFI loader advance to `0.2.2` without dependency upgrades.
All 20 local QEMU cases passed: 10 RISC-V boots, nine x86 UEFI boots, and one
release-profile x86 ISO boot. The ISO smoke kernel enables `qemu-debug-port`
for log capture and is not byte-identical to the ordinary release kernel.

Clippy passes with `-D warnings` for both kernel targets, every CI probe
configuration, and the release UEFI loader. Debug/release freestanding x86 ELF
checks, shell syntax checks, smoke-log parser self-tests, and diff checks pass.
These are local results, not a completed hosted GitHub Actions run.

The release ISO of this implementation booted on
physical x86-64 hardware and confirmed all three normal PASS markers followed
by the intended halt. That run preceded the version metadata bump. Dedicated
protection-fault probes remain QEMU validation.

## Concurrent execution remains outside the supported milestone

Execution remains one active CPU and one integer-only resident task. An
existing opposite-endpoint channel-send lock inversion under concurrent SMP
execution remains unresolved. Concurrent revoke calls are not linearized.
Neither behavior is exercised or supported by the current milestone, and
channel operations remain unwired from userspace.

Timer interrupts, preemptive/SMP scheduling, FP/vector user state, concurrent
unmapping, recoverable copy faults, ELF loading, init, musl, and complete
task/exit teardown remain pending. The physical allocator still manages at
most 4096 MiB of RAM. Node ownership and descendant revocation do not expand
those runtime guarantees.
