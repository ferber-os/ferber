# Uranium-238 v0.2.11

*Userspace can create, read and write owned-RAM VMOs through capability handles. Byte copies validate complete ranges and stage their data; allocation charges survive aliases, transfer and queued ownership. Two isolated processes verify shared backing and read-only enforcement. Uranium-238 still decays; closing a handle is not a half-life.*

## Three syscalls expose fresh memory without physical authority

Syscall 11 creates a zeroed VMO and returns its handle. Calls 12 and 13 read
and write exact byte ranges, returning zero only after complete success.
Logical size remains distinct from page-rounded backing. Creation accepts
1 byte through 16 MiB; each copy is bounded to 64 KiB. Options must be zero,
and physical/MMIO VMOs are excluded from this byte ABI.

Created capabilities receive READ, WRITE, DUPLICATE, TRANSFER, INSPECT and
MANAGE. They receive no MAP or EXECUTE authority. Existing handle operations,
attenuation, subtree revocation and channel move semantics continue to apply.
The concrete VMO object remains shared by aliases and derived capabilities;
generic inspection retains its existing type/rights format.

## Copies stage before committing and charges follow backing

A per-object content lock serializes kernel byte operations while size and
frame ownership remain immutable. Complete logical ranges, frame aliases and
user mappings are validated before a fallible heap staging buffer is used.
Writes snapshot the user source before changing backing; reads snapshot the
VMO before changing user output. Physical overlap is handled without violating
nonoverlapping-copy contracts. Contradictory physical aliases within a read
output are rejected using the shared channel-output validator.

Returned validation and allocation errors leave destinations unchanged.
Unpublished creation failures unwind frames, object/node ownership and budget
charges. Final ownership is destroyed after process, table and content guards
are released. These guarantees depend on stable mappings; they are not atomic
visibility guarantees against future mapped writers or other CPUs.

User-created backing is charged in pages against the smaller of 64 MiB and
one quarter of managed pages. The reservation belongs to the VMO until final
backing destruction. Duplication, derivation, transfer, queued messages and
creator exit neither recharge nor prematurely release it. Kernel-created
VMOs remain uncharged. This is a global bound on the new allocation surface,
not enforced hierarchical Job quotas or reserved kernel headroom. The roadmap
now distinguishes existing Job policy fields from implemented accounting.

## The external fixture proves sharing across processes

The separate `userspace-vmo-probe` configuration loads a producer and consumer
from an independently compiled Rust ELF. The producer checks zeroing, bounds
and errors, duplicates and derives a read-only child, then transfers it through
a channel. The consumer reads the bytes and cannot write through that child.
A producer update remains visible after the producer closes its owner and
exits; the consumer then releases final ownership.

Kernel evidence requires successful VMO, capability and channel operations,
two clean exits and no leaked handles. A warmup and three measured batches
restore frame, node, process, scheduler and user-budget baselines. The required
markers are `memory_verification`, `vmo_syscalls` and `boot_payload`, each with
the `[FERBER_TEST]` prefix and PASS result. The published release ISO continues
to select minimal init; `VMO_PROBE=1` builds the dedicated verification ISO.
Startup ABI v1 and init's START/READY protocol are unchanged.

## Validation covers boundaries, failures and physical execution

Local validation passed all 29 RISC-V and 25 x86 UEFI smoke cases, plus the
final release VMO ISO boot through OVMF. All 64 host tests passed: 17 memory,
12 channel, 15 ELF parser, seven loader/startup, eight UEFI payload, two init
protocol, two VMO core and one production VMO syscall allocation test.
Both kernel targets passed default and VMO-feature Clippy with warnings
denied; shell syntax and strict log-parser self-checks passed.

Release preflight repeated all 64 host tests, minimal-init and VMO-feature
Clippy on both kernel targets, release UEFI Clippy, shell/parser checks and a
freshly built `0.2.11` minimal-init release ISO boot through OVMF. The release
ELF shape check passed.

Three independent QA reviewers found no confirmed implementation bugs.
Additional adversarial checks covered byte-copy boundaries, output-alias
validation, every intermediate frame-allocation failure, final-owner release
and queued revoked descendants. One consolidated error-table documentation
finding was fixed and rechecked. Host harnesses substitute allocation or
translation where required; those results do not replace hardware execution.

A physical x86-64 boot on the 32 GiB MSI workstation reached all three VMO
markers with the pre-bump `0.2.10` banner. That demonstrates the two-process
exchange, denied read-only write, shared updates and repeated reclamation and
budget checks. Hosted Actions results remain separate from local evidence.

The kernel and UEFI loader advance to `0.2.11` without dependency upgrades.
This remains a prerelease; `0.3.0` stays reserved. User mapping/unmapping,
executable publication/sealing and hierarchical Job charging remain pending.
Execution remains cooperative, integer-only and single-CPU with interrupts
masked. Content locks do not establish mapped-writer, DMA, interrupt-reentrancy
or SMP safety, and minimal init still has no service-launching event loop.
