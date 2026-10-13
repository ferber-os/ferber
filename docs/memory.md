# Wolfram Memory Model

Wolfram is a "VMO-everything" system.

## Virtual Memory Objects (VMOs)
Every region of virtual memory is backed by a named kernel object called a VMO.
- **No Anonymous Memory:** There is no implicit backing. If you allocate memory, you are creating or mapping a VMO.
- **Hardware Enforcement:** Rights (READ, WRITE, EXECUTE) are enforced at the page table level by the MMU. This means capability rights translate directly into hardware protection.

## Physical Allocation
The physical memory allocator operates in two phases:
1. **Boot:** A simple bitmap allocator (Phase 1) manages initial setup.
2. **Runtime:** A buddy allocator (Phase 2) takes over for dynamic runtime allocation, supporting page splitting and coalescing.

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
  - Teardown safety: `Drop` only traverses user entries 0..255, preserving shared kernel tables.
  - CR3 activation with `invlpg` single-page and full CR3 reload TLB flushes.

*See [docs/architecture.md](architecture.md) for architectural context.*
