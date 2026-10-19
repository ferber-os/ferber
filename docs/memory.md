# Ferber Memory Model

Ferber is a "VMO-everything" system.

## Virtual Memory Objects (VMOs)
Every region of virtual memory is backed by a named kernel object called a VMO.

- **No Anonymous Memory:** There is no implicit backing. If you allocate memory, you are creating or mapping a VMO.
- **Hardware Enforcement:** Rights (READ, WRITE, EXECUTE) are enforced at the page table level by the MMU. This means capability rights translate directly into hardware protection.
- **No Stale Allocation Contents:** Allocated VMOs clear every physical frame before exposure, including partial-page tails. Physical/MMIO VMOs preserve the existing contents of the physical range.
- **Explicit Lifetime:** Page-table mappings do not own or free mapped frames. Their VMO owners must outlive every use of those mappings.

## Physical Allocation
The bitmap allocator owns physical page allocation. The runtime buddy heap
suballocates blocks from those pages; requests larger than one page use the
bitmap allocator's contiguous-frame path.

Discovery validates and normalizes the boot map before the allocator uses it:
usable ranges are merged, rounded inward to complete pages, and reduced by
outward-rounded reservations. Reservations win overlaps. Holes consume neither
frames nor bitmap storage. Kernel, firmware, handoff, framebuffer, FDT and
page-zero reservations remain unavailable. UEFI currently treats only
`EFI_CONVENTIONAL_MEMORY` as usable; loader, boot-service, runtime, and ACPI
memory are not reclaimed. Reclaiming boot-services pages is a future memory
efficiency step and must wait until loader ownership and all references to
those pages have ended. FDT discovery honors enabled memory nodes and static
reservations, including `no-map`; unsupported reserved-memory translations
and unresolved dynamic reservations stop boot explicitly.

Each bitmap block occupies one independently carved page from the validated
normalized extents. Bootstrap placement therefore depends on correct map
validation: metadata pages are carved and permanently reserved before those
extents become generally allocatable. Host tests cover map normalization and
reservation subtraction; boot accounting checks
`managed = bootstrap-reserved + allocated + free`; the printed start/end
envelope is not a capacity measurement. A failed free validates the entire
range before changing ownership. Contiguous allocations can cross bitmap
blocks, but cannot cross physical holes. The heap's 128 MiB single-allocation
limit remains separate from total physical capacity.

The allocator's atomic spinlock serializes bitmap state, but it is not
interrupt-reentrant. If an interrupt handler tries to allocate while the
interrupted context holds that lock, the CPU spins on its own lock forever.
Allocation from interrupt context therefore requires an IRQ-safe locking
policy first. SMP also requires a reviewed synchronization and initialization
publication model before multiple CPUs use the allocator; secondary harts
currently remain parked.

The current kernel-stack implementation requires physically contiguous VMOs.
It uses the first frame's kernel virtual alias plus the VMO size as the stack top, so it
cannot fall back to scattered pages. User stacks may use scattered VMO frames
mapped into a contiguous virtual range.

Contiguous physical VMO backing stores a base and page count instead of a
vector entry per page. Physical VMOs expose frames for explicit mappings;
direct CPU read/write requires a RAM alias or an explicit device alias.
The GOP framebuffer has a separate device alias and uses volatile accesses.
Unknown device ranges do not become ordinary RAM aliases.

## Paging HAL & Architecture Parity

Hardware page tables are managed through the architecture-agnostic `AddressSpace` trait (`ferber/src/kernel/memory/paging.rs`). The kernel core never manipulates hardware page table entries directly.

- **RISC-V 64 (Sv39):**
  - 3-level page table walk (`VPN[2..0]`).
  - Canonical 39-bit sign-extension verification (bits 63..39 match bit 38).
  - PTE bitfield flags (`V, R, W, X, U, G, A, D`) enforcing $W \implies R$.
  - 44-bit PPN positioning (bits 53..10).
  - `satp` CSR activation (`MODE=8`) with single-page and global `sfence.vma` TLB flushes.
- **x86-64 (PML4):**
  - 4-level page table walk (`PML4 -> PDPT -> PD -> PT`).
  - Canonical 48-bit sign-extension verification (bits 63..48 match bit 47).
  - PTE flags (`P, RW, US, PWT, PCD, A, D, HUGE, G, NX`).
  - Upper-half kernel space synchronization: PML4 entries 256..511 copied from the permanent kernel root.
  - Teardown safety: `Drop` traverses only owned lower-half entries 0..255, preserving shared kernel tables and all leaf backing frames.
  - Queries combine ancestor permissions: USER and RW must hold throughout the walk, while NX at any level disables execution. Huge-page queries preserve byte offsets.
  - CR3 activation with `invlpg` single-page and full CR3 reload TLB flushes.

## Permanent upper mappings keep the kernel reachable

The resident user code VMO starts at `0x0000_0010_0000_0000` and is user RX.
Its backing rounds the actual probe length up to whole pages; it is not limited
to a single page. Its 16 KiB stack starts at `0x0000_0020_0000_0000` and is user
RW/NX. Both addresses are in the positive canonical Sv39 range and work on
x86-64 as well. The dedicated execute-fault probe maps its code RO/NX instead.

Both kernels execute at `0xffff_ffff_8000_0000` on high image-backed stacks
before entering Rust. RISC-V retains a small physical assembly entry at
`0x80200000`; x86 keeps its PIE/relative-relocation ELF contract while the
loader writes through physical destinations and calculates high pointer
values. BootInfo version 2 is 112 bytes and separates physical reservations
from readable virtual pointers, including the framebuffer alias.

Eligible RAM receives packed supervisor RW/NX aliases in
`[0xffff_ffc0_0000_0000, 0xffff_fffe_0000_0000)` on Sv39 and
`[0xffff_8000_0000_0000, 0xffff_fffe_0000_0000)` on x86-64. Physical holes
consume no alias capacity; 2 MiB alignment padding does. These virtual windows
are hard architectural capacity limits: Sv39 provides 248 GiB, while x86-64 is
bounded by its window and the CPU's physical-address width. A map that exceeds
either limit fails boot explicitly instead of truncating or silently omitting
RAM.
The last 2 GiB holds the image, with the preceding 6 GiB reserved for boot
windows and explicit MMIO. Checked translation helpers reject unknown RAM.

Before the bitmap can touch metadata, RISC-V installs temporary packed
aliases using image-owned tables. x86 uses a bounded scratch mapping to
build aliases even for physical RAM outside its temporary identity range.
Both then construct exact permanent tables and remove temporary identities,
scratch aliases, and rounded bootstrap mappings before ordinary heap/VMO use.
The kernel image is excluded from writable RAM aliases: text is RX, rodata
RO/NX, and data/stacks RW/NX. Reserved and `no-map` memory has no ordinary
RAM alias. Permanent RAM, image, and retained boot/device mappings have
compact physical VMO backing.

New process roots have empty lower halves and shared permanent upper halves.
Map/unmap/protect reject modifications to shared upper entries. Teardown
frees private user tables and roots while retaining kernel tables and leaf
backing. Cooperative exit switches to the permanent-root dispatcher stack
before retiring the thread and, for its last thread, the process. Code and
stack VMOs are released outside scheduler/process locks after their mappings
and active execution no longer need them. The dedicated fatal fault probes
retain their backing through halt. SMP remains separate work.

`memory_verification: PASS` checks high code/stack permissions, absence of
the low kernel alias, shared-upper mutation rejection, and accounting. It walks
detected RAM extents in order and verifies alias translation and supervisor
RW/NX permissions at both ends of each extent. An ordinary owned VMO page
exercises allocation, initial zeroing, user mapping, read/write and user copies
after address-space activation, unmapping, and free/allocated-page accounting
restoration. Only that owned allocation is written. This is representative
backing verification, not an exhaustive destructive RAM test. The same checks
run regardless of physical addresses or total RAM; no threshold-specific
capacity build is required. Host memory/FDT tests run with
`bash scripts/memory-tests.sh`.
The current x86-64 release ISO has also passed these checks on physical
hardware with 32 GiB installed: 8,240,704 managed frames across 23 extents.

## ELF segments retain their own VMO backing

The opt-in static ELF loader validates the entire image before creating a
process. Each `PT_LOAD` gets a separate zeroed VMO rounded outward to pages;
file bytes are copied at the segment's in-page offset, leaving BSS and page
padding zero. Mappings allow only user R-X, R--, or RW- permissions. Segments
cannot share a rounded mapping page or overlap the fixed stack/guard range.
Process-owned VMOs retain segment backing until teardown; page tables still
never own leaf frames.

The thread retains a separate zeroed 16 KiB user stack at
`0x0000_0020_0000_0000`, mapped RW/NX, with the preceding page unmapped. Entry
starts at the aligned stack top without arguments, environment, or auxiliary
vector. Partial construction is never runnable and is detached from the Job
on failure. The ELF probe repeats execution after warming allocator capacities
and checks frame, capability-node, Job process-count, and scheduler baselines.
See [the ELF loading contract](elf-loading.md) for limits and delivery.

## Resident user copies preflight the whole range

`copy_from_user` and `copy_to_user` return `InvalidRange`, `NotMapped`, or
`PermissionDenied`. Zero-length operations succeed without dereferencing the
address. Nonempty operations reject null, overflow, and addresses outside the
lower canonical user range: through `0x3f_ffff_ffff` on Sv39 and
`0x7fff_ffff_ffff` on x86-64.

Before writing a destination, the helpers query every touched page for
effective USER and READ or WRITE permissions. An invalid later page leaves
the destination unchanged. Copies then use supervisor physical aliases one
page at a time, with the translated byte offset applied once. RISC-V SUM is
unchanged.

These are unsafe APIs. Callers must keep mappings and VMO backing stable
until return, guarantee readable or writable supervisor RAM aliases, and
ensure the kernel buffer does not overlap the user backing. This resident
contract provides no demand paging, concurrent-unmap handling, or fault
fixup. Boot checks exercise cross-page success, invalid ranges, missing and
restricted pages, permission aggregation, unchanged destinations after failed
preflight, and frame accounting after teardown.

*See [docs/architecture.md](architecture.md) for architectural context.*
