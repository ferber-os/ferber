# Uranium-238 v0.2.13

*Userspace can write code into a VMO, seal its backing irreversibly, and execute it through an RX mapping on both architectures. Writable aliases anywhere block publication; old WRITE rights cannot reopen sealed memory. Uranium-238 decays. This state transition does not.*

## Executable authority is explicit before sealing

Syscall 16, `sys_vmo_seal(handle, options=0)`, requires MANAGE and EXECUTE
through a live owned-RAM VMO capability. It seals the entire object, allocates
nothing and succeeds again when called with valid authority on sealed backing.
There is no unseal, partial seal or implicit mapping conversion.

Creation options 0 and 1 preserve their previous rights exactly. The new
`VMO_CREATE_EXECUTABLE` bit requires `VMO_CREATE_MAPPABLE`; options 3 grant
MAP and EXECUTE in addition to the original rights. Option 2 alone and unknown
bits are invalid. Sealing never manufactures capability rights or replaces a
handle. Existing aliases keep their rights bits and share the object's state.

Syscall 14 accepts permission 5 for RX, requiring READ, MAP, EXECUTE and sealed
backing. RO remains available on mutable or sealed objects; RW/NX requires
mutable backing. RWX and execute-only requests remain invalid. Existing size,
logical-page, protected-range, mapping-record and private-table limits remain
unchanged. Physical/MMIO and nonowning VMOs remain excluded.

## Writable mappings retain permits through invalidation

The VMO's content lock now protects sealed state and an object-wide count of
writable-mapping permits. A writable mapping in any process blocks seal with
`-16`, even after its originating capability is closed, transferred or revoked.
RO mappings may remain installed. A writable subrange blocks the whole object;
publication does not silently remove another process's access.

RW admission acquires a nonallocating RAII permit before fallible mapping
preparation. Failed allocation or quota admission releases it. A successful
mapping record retains it until exact unmap or process teardown invalidates
the translations. Invalid or partial unmap cannot release it. Killing a task
only marks it dead; dispatcher reaping and fallback destruction preserve the
address-space-before-owner ordering.

After sealing, writes through surviving WRITE handles, new RW mappings and
ordinary internal VMO write methods return permission failure. Empty writes
are denied too. The syscall checks eligibility before user preflight or staging;
the write method checks again under the shared state lock when committing.
Reads and ordinary capability ownership operations retain their existing rules.
Revocation still does not remove installed mappings.

Kernel output copies continue requiring effective writable user pages. VMO
read destinations, capability inspection and channel byte/handle/count outputs
cannot write through RO or RX aliases. Denied receives preserve the queued
message and handle ownership. The guarantee covers supported user-controlled
mutation paths; trusted supervisor RAM aliases remain RW/NX.

## RX installation publishes instructions locally

Both production walkers perform an allocation-free instruction-publication
step after committing executable leaves and flushing local translations.
RISC-V executes `fence.i` with compiler-visible memory effects; x86-64 executes
CPUID leaf zero between compiler fences. This orders code written through the
supervisor RAM alias before fetching it through the user mapping.

The initial RISC-V user-entry fence alone does not cover executable code
installed during later returning syscalls. Failed maps and non-executable maps
skip the new publication hook. Trusted ELF RX installation uses the same
walker hook without changing loader ownership or adopting user sealing charges.
Local publication is not remote instruction synchronization or an SMP shootdown.

## The fixture calls the bytes it publishes

The separately compiled sealing fixture contains relocation-free leaf functions
returning 17 and 42. It copies their bytes through syscall 13, seals the VMOs,
maps RX and calls them in user mode. Its peer holds a writable alias to prove
publication remains blocked after source-handle closure and revocation, then
unmaps explicitly so sealing can proceed.

The producer and consumer execute shared sealed backing, retain executable
mappings after handle closure/revocation and report actual return values through
channels. The producer replaces function A with separately sealed function B
at the same VA in an already-running process and observes 42 instead of 17.
Kernel evidence requires the result messages, successful seal/map operations,
two clean exits and drained records. Warmup plus three measured batches restore
frame, node, process, scheduler and user-backing-budget baselines.

`VMO_SEAL_PROBE=1` builds the dedicated verification ISO. Its required markers
are `memory_verification`, `vmo_sealing` and `boot_payload`, each with the
`[FERBER_TEST]` prefix and PASS result. A separate sealed-RX-write build requires
a matching user-origin protection fault, including the known instruction,
address, hardware cause and retained sealed RX metadata. Its marker is
`vmo_sealing_write_rx`; that deliberate fault halt does not prove reclamation.
Existing RO-write and NX-execute probes remain separate regressions.

## Validation includes adversarial review and physical x86 execution

Post-interruption implementation validation passed all 41 RISC-V and 35 x86
QEMU smoke cases, plus the release sealing ISO boot through OVMF's CD-ROM path.
All 80 host tests passed: 17 memory, 12 channel, 15 ELF parser, seven loader/startup,
eight UEFI payload, two init protocol, two VMO core, one production VMO syscall,
seven production Sv39 walker and nine production PML4 walker tests. Default
and all 11 user-probe/deadlock feature Clippy configurations passed on both
kernel targets with warnings denied, as did release UEFI Clippy. Both fixture
modes built for both architectures in debug and release; shell syntax and the
strict log-parser self-check passed.

Release preflight repeated all 80 host tests and the parser self-check.
After the version bump, fresh `0.2.13` minimal-init and sealing release ISOs passed
OVMF boots and freestanding ELF shape checks. Default, minimal-init, normal
sealing and sealed-write-fault Clippy passed on both kernel targets, together
with release UEFI Clippy, all with warnings denied.

Four independent reviewers examined the patch: red team, blue team, contract
lawyer and failure gremlin. No confirmed runtime bugs were found. One missing
consolidated syscall error-table entry was fixed and rechecked. Additional
adversarial checks covered rights subsets, options, permission encodings,
publication recovery, provisional permit rollback, alias quotas, retained dead
processes and final ownership. A blue-team mutation check verified that the
regressions detect premature writable-permit release. Host harness substitutes
and instrumented walker results remain distinct from actual CPU execution.

A physical x86-64 boot on the 32 GiB MSI workstation reached all three sealing
markers with the pre-bump `0.2.12` banner and 31.46 GiB managed RAM. That run
includes actual RX execution, same-address code replacement, cross-process
writable-alias blocking and repeated reclamation checks. The sealed-RX-write
fault probe remains QEMU-validated. Physical RISC-V hardware has not been
available for validation. Hosted Actions results remain separate from local
evidence.

The QA protocol now includes the independent blue-team defensive-control lens.
A dedicated Easter egg engineer protocol permits fitting comments, interactions
and runtime behavior with ordinary safety and verification requirements. This
patch adds one comment beside the seal transition and registers it in the
encoded contributor guide: WRITE bits on old handles are now collector's items.

The kernel and UEFI loader advance to `0.2.13` without dependency upgrades.
The published ISO continues to boot minimal Ferrum init; startup ABI v1 and its
readiness protocol are unchanged. This remains a prerelease; `0.3.0` stays
reserved. Userspace task creation, service launch, protection changes, partial
unmap and hierarchical Job charging remain pending. Execution is cooperative,
integer-only and single-CPU with interrupts masked. Raw kernel writes, DMA,
concurrent mapped writers, interrupt reentrancy, CPU migration and remote
instruction/TLB synchronization remain outside these guarantees.
