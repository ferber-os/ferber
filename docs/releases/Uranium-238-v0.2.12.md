# Uranium-238 v0.2.12

*Userspace can map owned-RAM VMOs read-only or read/write, share their backing across isolated processes, and unmap exact regions. Both paging implementations prepare new tables before publishing mappings and reclaim empty private tables after invalidation. Uranium-238 still has a longer lifetime than a closed handle.*

## MAP authority is explicit and mappings remain non-executable

`sys_vmo_create(size, 0)` preserves the byte API's original rights. The new
`VMO_CREATE_MAPPABLE` option adds MAP; no option grants EXECUTE. Syscall 14 maps
a page-aligned logical subrange at an explicit address. RO requires MAP and
READ; RW/NX additionally requires WRITE. Syscall 15 removes one exact mapping
record belonging to the caller. It needs no surviving handle or MANAGE right.

Maps exclude physical/MMIO and nonowning VMOs, partial logical pages,
noncanonical or out-of-user-range addresses, registered stacks and their
guards. Existing leaves, huge mappings and reserved nonzero entries cannot be
overwritten. Partial unmaps, combined records and loader/stack unmaps fail.
The existing byte-copy ABI and startup ABI v1 remain unchanged.

Each process is limited to 64 user mapping records, 16,384 mapped pages
(64 MiB, counting aliases) and 256 private table frames, including its root
and loader/stack tables. Each map remains bounded to 16 MiB. The existing
user-created backing budget is separate and follows the VMO's lifetime.

## Records own backing independently of capability nodes

A successful mapping retains an `Arc<Vmo>` together with its virtual range,
VMO offset and permissions. It does not retain the capability node or its
ancestry. Closing, transferring or revoking a handle leaves existing mappings
valid. Revocation still rejects later operations through that node; it does
not withdraw installed memory access. RO describes this mapping's permitted
access, and does not promise an immutable object when other writers exist.

Unmap validates every expected frame and permission before changing tables.
Backing stays owned until translation invalidation completes and is released
outside the process lock. Last-thread reaping drains dynamic mappings from
the permanent-root dispatcher stack, including when another kernel reference
retains the dead Process. Killing a process only marks its tasks dead and
never tears down the active address space in place.

## Page-table changes publish only after preparation succeeds

Sv39 and PML4 preflight the complete raw range and count missing table
prefixes. New table frames are allocated and zeroed off-tree, with record
capacity and quotas checked before reachable PTEs change. Returned preparation
errors leave existing mappings and effective permissions unchanged. The final
publication path allocates nothing. x86 ancestor permission changes occur
only during that commit and preserve existing sibling restrictions.

Exact unmap clears validated leaves and prunes empty owned lower-half tables.
Reserved entries and other mappings prevent pruning. Both ports detach links
and flush local translation state before freeing retired table frames. Roots,
shared upper-half tables and VMO data frames are excluded from that pruning.
These operations retain the current single-active-CPU, interrupts-masked
contract; local flushes are not SMP shootdowns.

## The external fixture exercises real mapped access

The independently compiled producer and consumer map shared backing, access
it through volatile loads/stores and transfer a READ/MAP capability through a
channel. Mappings survive handle close, updates remain visible across
processes, and unmap/remap at the same address checks replacement backing.
Kernel evidence requires successful mapping and channel operations, two clean
exits, drained records and repeated restoration of frame, node, task and
backing-budget baselines after warmup.

The dedicated `VMO_MAP_PROBE=1` ISO requires `memory_verification`,
`vmo_mapping` and `boot_payload` PASS markers. Separate RO-write and NX-execute
builds require diagnosed user faults with the expected origin, instruction,
address and mapping metadata. Their deliberate fault halts do not claim task
reclamation. The published ISO continues to boot minimal Ferrum init.

## Validation includes failures and physical x86 execution

Implementation validation passed all 36 RISC-V and 31 x86 QEMU smoke cases,
plus the release mapping ISO through OVMF's CD-ROM path. All 78 host tests
passed: 17 memory, 12 channel, 15 ELF parser, seven loader/startup, eight UEFI
payload, two init protocol, two VMO core, one production VMO syscall, six
production Sv39 walker and eight production PML4 walker tests. Default and
all three mapping-feature kernel Clippy configurations passed on both ports
with warnings denied. External fixture builds, shell syntax and strict log
parser self-checks passed.

Post-bump release preflight passed fresh `0.2.12` minimal-init and mapping
ISO boots through OVMF, release UEFI Clippy with warnings denied, and the
strict log-parser self-check. Both ISO builds passed the freestanding ELF
shape check.

Three independent QA reviewers found no confirmed implementation bugs.
Additional adversarial checks covered intermediate allocation failures,
unchanged PTE snapshots on failure, raw reserved entries, table boundaries,
alias-page quota exhaustion, backing pinning and cleanup. Host walkers use
substituted allocation/translation support; QEMU provides separate evidence
for actual local translation and userspace access.

A physical x86-64 boot on the 32 GiB MSI workstation reached all three mapping
markers with the pre-bump `0.2.11` banner and 31.46 GiB managed RAM. Dedicated
RO-write/NX-execute fault probes remain QEMU-validated. There has been no
physical RISC-V validation. Hosted Actions results remain separate from local
evidence.

The kernel and UEFI loader advance to `0.2.12` without dependency upgrades.
This remains a prerelease; `0.3.0` stays reserved. Executable publication,
protection changes, partial unmap, userspace task creation and hierarchical
Job charging remain pending. Execution remains cooperative, integer-only and
single-CPU with interrupts masked. Content locks do not serialize mapped
writers, and mapped-writer, DMA, interrupt-reentrancy and SMP safety remain
outside this release's guarantees.
