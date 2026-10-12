# Uranium-238 v0.1.3

This patch release hardens kernel heap allocation boundaries and resolves test harness buffering discrepancies across emulator environments.

## Included

- **Dynamic Kernel Versioning:** Dynamically derived the kernel startup greeting banner from `CARGO_PKG_VERSION`, bringing the banner in line with release `0.1.3`.
- **Heap Allocation Upper Bound:** Enforced an explicit single-allocation ceiling (`MAX_ALLOC_SIZE = 128 MiB`) in `BuddyAllocator::alloc`. Allocations exceeding this threshold cleanly return `null` instead of forwarding unbounded multi-page requests to the physical frame allocator.
- **Deterministic OOM Test:** Hardened the kernel's heap out-of-memory verification using `core::hint::black_box` to ensure the test reliably verifies allocation failure across all memory configurations and compiler optimization levels.
- **Smoke Harness Stream Buffering:** Configured `scripts/riscv-smoke.sh` to route emulated serial console output directly to the trial log file (`-serial "file:$log"`), preventing glibc block-buffering from dropping early diagnostics when QEMU is terminated by `timeout`.
