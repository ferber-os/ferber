# Wolfram Memory Model

Wolfram is a "VMO-everything" system.

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

The current kernel-stack implementation requires physically contiguous VMOs.
It uses the first physical frame plus the VMO size as the stack top, so it
cannot fall back to scattered pages. User stacks may use scattered VMO frames
mapped into a contiguous virtual range.

## Paging HAL & Architecture Parity

Hardware page tables are managed through the architecture-agnostic `AddressSpace` trait (`wolfram/src/kernel/memory/paging.rs`). The kernel core never manipulates hardware page table entries directly.

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
  - Upper-half kernel space synchronization: PML4 entries 256..511 copied from boot CR3.
  - Teardown safety: `Drop` traverses only owned lower-half entries 0..255, preserving shared kernel tables and all leaf backing frames.
  - Queries combine ancestor permissions: USER and RW must hold throughout the walk, while NX at any level disables execution. Huge-page queries preserve byte offsets.
  - CR3 activation with `invlpg` single-page and full CR3 reload TLB flushes.

## The bootstrap address space keeps the kernel reachable

The resident user code VMO starts at `0x0000_0010_0000_0000` and is user RX.
Its backing rounds the actual probe length up to whole pages; it is not limited
to a single page. Its 16 KiB stack starts at `0x0000_0020_0000_0000` and is user
RW/NX. Both addresses are in the positive canonical Sv39 range and work on
x86-64 as well. The dedicated execute-fault probe maps its code RO/NX instead.

On RISC-V, each bootstrap address space identity-maps the allocator-managed
RAM bank through a retained physical VMO. Mappings begin supervisor RW/NX;
page-aligned kernel text becomes supervisor RX and rodata becomes supervisor
RO/NX. This keeps the kernel, heap, stack, and physical page-table aliases
reachable after `satp` activation. User mappings remain separate. The kernel
executes `fence.i` after installing the probe code.

On x86-64, architecture initialization captures the loader-owned boot CR3
before any process root can become active. Address-space creation privately
clones its lower-half table pages and shares its upper-half entries. Cloning
preserves physical leaves, huge pages, permissions, and cache attributes.
Opening an ancestor for a new user mapping preserves the restrictions on
existing sibling mappings. The identity-mapped kernel leaves remain
supervisor-only. Partial construction and teardown free owned table pages
without freeing the loader's tables or VMO backing.

Both layouts are bootstrap shortcuts pending permanent kernel virtual
mappings. The retained bootstrap context owns the root Job, process, thread,
code, and supervisor backing; the thread owns its kernel and user stack VMOs.
No owner is dropped while its page tables or stack remain active. Exit marks
the bootstrap thread Dead and halts; it does not yet reclaim the active task.

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
