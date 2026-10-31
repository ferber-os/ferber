# Uranium-238 v0.2.14

*Physical allocation charges now follow their original Job budget through transfer, creator exit and final destruction. Hierarchical admission checks prevent descendants from exceeding ancestor limits.*

## Job limits include their descendants

The previous Job limit and usage fields become enforced allocation budgets.
Admission checks the complete retained ancestry before changing counters; an
exhausted or closed ancestor rejects the reservation without a partial charge.
The bootstrap root Job receives half of managed physical RAM as its limit,
with at most 32 child edges below it.

A System budget accounts for bootstrap reservations, permanent kernel tables
and kernel-owned allocations. Its inclusive usage also contains Job charges,
so System and Job totals must not be added together. System has no policy quota
and can still encounter physical exhaustion. Bootstrap reservations are adopted
once after the physical allocator initializes. Native checks reconcile System
usage with bootstrap-reserved and allocated frame totals.

## Allocator overhead belongs to an owner

The buddy allocator now uses separate arenas per budget. Charges count complete
physical backing pages, including ownership headers, alignment padding, unused
blocks and fragmentation. Fully coalesced pages return to the bitmap allocator
and release their charge. Large allocations charge the complete rounded span.
The payload ceiling remains 128 MiB and supported alignment remains at most
4 KiB; a page-aligned 4 KiB payload can consume two pages with its header.

Allocation headers retain the original budget. Freeing from another allocation
context refunds that owner. Reallocation preserves ownership, accounts for old
and replacement storage concurrently when replacement is required, and preserves
the old allocation on failure. Budget metadata is allocated under the parent
budget so the budget cannot own its own allocation.

Process construction, thread stacks, ELF population, capability nodes, syscall
buffers and mapping metadata now participate in allocation accounting. Channel
queue and waiter growth uses the channel's original budget; message buffers
retain their own owners. Target task records belong to their Job, while global
scheduler storage and the dispatcher stack are explicitly System-owned. Job,
process and thread ID allocation also rejects exhaustion instead of wrapping.

## Closing authority does not refund live backing

Owned VMOs retain frame charges independently of their capabilities. Transfer,
duplication, attenuation, close and revocation do not move those charges.
Mappings and queued capabilities can keep backing alive after its creator exits.
Physical/MMIO VMOs do not charge device addresses as owned RAM; their metadata
still uses accounted heap storage.

Killing a Job closes its budget before cascading termination. Closed ancestry
rejects new allocations, including reuse of cached buddy blocks. Existing memory
remains usable and reclaimable. Retained budget ancestry survives the structural
Job until its final allocation disappears. Owned frames are freed before their
charge is released; private page-table removal flushes local translations before
freeing tables and refunding them on both architectures. Failed preparation
unwinds provisional charges without publishing partial mappings.

Fallible syscall allocation paths distinguish quota or closed-ancestry denial
(`-28`) from physical or allocator exhaustion (`-12`). The existing global
user-created VMO backing limit remains an additional policy counter, rather
than a second charge for the same physical pages. Infallible Rust allocation
paths can still panic on exhaustion. No new syscall numbers are introduced.

## Allocation context follows execution

The cooperative dispatcher selects the running process's budget and returns to
System ownership before dispatcher work and reclamation. Scoped owner overrides
and allocation-failure capture must finish before a task switch; assertions
enforce that boundary. The ledger is serialized, but the production context
still assumes one active CPU with interrupts masked. Host concurrency tests use
thread-local contexts and do not establish production IRQ or SMP safety.

## The probe keeps the charge after killing its creator

The separately compiled accounting fixture runs producer and consumer processes
under sibling Jobs with 4 MiB limits and an 8 MiB parent limit. Each encounters
a Job-denied VMO creation. The producer then creates and writes an 8 KiB VMO,
derives a read/transfer capability and sends it to the consumer. After producer
exit and Job closure, the consumer reads the surviving backing while evidence
checks that the original closed budget retains the charge. Final foreign release
restores backing capacity. Warmup plus three measured batches restore frame,
capability-node, Job/process, scheduler and allocation-budget baselines.

`JOB_ACCOUNTING_PROBE=1` builds the dedicated verification ISO. Its required
markers are `memory_verification`, `job_accounting` and `boot_payload`, each
with the `[FERBER_TEST]` prefix and PASS result. Ordinary boots also run native
checks against the real global allocator, including closed budgets, ancestor
denial, foreign destruction and accounting restoration.

## Validation covers both ports and physical x86

Before the version bump, all 93 host tests passed: 17 memory, 13 channel,
15 ELF parser, seven loader/startup, eight UEFI payload, two init protocol,
two VMO core, one VMO syscall, eight Sv39 walker, ten PML4 walker and ten
production ledger/allocator tests. All 44 RISC-V and 37 x86 QEMU smoke cases
passed, together with all seven release ISO scenarios through OVMF's CD-ROM
path. Default and probe-feature Clippy configurations passed on both kernel
targets, as did release UEFI Clippy, with warnings denied. Shell syntax,
strict log-parser self-checks and diff checks passed.

After the version bump, fresh accounting and minimal-init release ISOs passed
OVMF boots and freestanding ELF validation. Accounting and minimal-init Clippy
passed on both kernel targets, together with release UEFI Clippy, all with
warnings denied. The ten accounting host tests, strict log-parser self-check
and manifest/lockfile consistency checks also passed again.

Additional adversarial checks exercised 30,000 mixed allocator operations across
seven bounded owners, 12,000 operations with foreign-context frees, 260 payload
size/alignment combinations, and forced allocation/reallocation failures across
40 combinations. A temporary weakened closed-ancestor check was detected by the
cached-arena regression. The identified regression was fixed.
Host metadata allocation and instrumented paging remain distinct from native
execution evidence.

A physical x86-64 boot on the 32 GiB MSI B650M-A PRO WIFI reached all three
accounting markers, with native hierarchical accounting checks reporting OK
and 31.46 GiB managed RAM. That run used the pre-bump `0.2.13` banner. Physical
RISC-V hardware has not been available; RISC-V evidence remains QEMU-only.
Hosted Actions results remain separate from these local results.

The kernel and UEFI loader advance to `0.2.14` without dependency upgrades.
The published ISO continues to boot minimal Ferrum init using startup ABI v1.
This remains a prerelease; `0.3.0` stays reserved. Userspace task creation,
service launch, preemption, IRQ-safe allocation and SMP remain pending. The
accounting contract is documented in [Job accounting](../reference/job-accounting.md).
