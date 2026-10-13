# Uranium-238 v0.1.4

This release delivers the Paging Hardware Abstraction Layer (HAL), fulfilling Milestone 1 of Phase 2 (Kernel Core). It establishes full 1:1 architectural parity across RISC-V 64 and x86-64, providing virtual memory address spaces, dynamic multi-level page table walking, and hardware page table activation.

## Included

- **Unified Paging HAL Interface:** Implemented the architecture-agnostic `AddressSpace` trait in `wolfram/src/kernel/memory/paging.rs`, providing safe abstractions for `map_page`, `unmap_page`, `protect_page`, `query`, `activate`, and TLB shootdowns.
- **RISC-V 64 Sv39 Paging:** Implemented `Sv39AddressSpace` (`wolfram/src/arch/riscv64/paging.rs`) featuring a 3-level page table walk (`VPN[2..0]`), canonical 39-bit sign-extension verification, standard PTE bitfields (`V, R, W, X, U, G, A, D`) with $W \implies R$ rule enforcement, `satp` CSR activation (`MODE=8`), single-page and global `sfence.vma` TLB invalidation, and leak-free `Drop` teardown.
- **x86-64 PML4 Paging:** Implemented `Pml4AddressSpace` (`wolfram/src/arch/x86_64/paging.rs`) featuring a 4-level page table walk (`PML4 -> PDPT -> PD -> PT`), canonical 48-bit address validation, intermediate `P | RW | US` permission propagation, upper-half kernel synchronization (copying entries 256..511 from active CR3 root), CR3 activation, `invlpg` shootdowns, and kernel-safe teardown preserving shared entries.
- **In-Kernel Verification Suite:** Extended early boot diagnostics in `wolfram/src/main.rs` with live verification of dynamic VMO mapping (2 pages), virtual address translation queries, collision rejection (`AlreadyMapped`), in-place permission transitions (`user_rw` to `user_ro`), unmapping, and teardown across both architectures.
- **Test Harness Hardening:** Restored `scripts/x86-preview.sh` and verified repeatable multi-hart RISC-V QEMU boots (128 MiB & 256 MiB) and x86 UEFI QEMU boots (debug, release, and deliberate exception probe).

This release establishes the hardware address space foundation necessary to begin Milestone 2: the process and thread isolation containers.
