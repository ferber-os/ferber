# Uranium-238 v0.1.5

*Jobs, processes and threads now form an explicit lifetime hierarchy. Processes begin with empty handle tables, thread stacks have VMO backing, and both architectures support kernel context switching.*

## What changed

- **Job Tree & Zero Orphan Teardown:** Implemented singleton root job (`Job::create_root()`), sub-job hierarchy, child processes, and synchronous recursive teardown via `Job::kill()`, guaranteeing no orphaned processes or dangling resources.
- **Zero Ambient Authority:** Enforced microkernel security invariants where processes initialize with an empty `HandleTable` (0 handles), requiring explicit capability delegation with zero ambient descriptors or memory inheritance.
- **Dual-Stack VMO Architecture:** Backed thread execution environments with dedicated kernel and user stacks allocated directly through contiguous Virtual Memory Objects (`Vmo`).
- **Hardware Context Switching Parity:**
  - *RISC-V 64:* 112-byte `CpuContext` (`ra, sp, s0..s11`) assembly save/restore in `arch/riscv64/context.S` and `context.rs`.
  - *x86-64:* 56-byte `CpuContext` (`rsp, rbp, rbx, r12..r15`) assembly save/restore in `arch/x86_64/context.S` and `context.rs` with `cld` direction flag sanitization prior to `ret`.
- **Cooperative Scheduler Staging:** Integrated an O(1) ready queue backed by `alloc::collections::VecDeque` (`READY_QUEUE`) supporting `spawn`, `ready_count`, and `sched_yield`.
- **Boot Diagnostics & Live Verification:** Completed comprehensive 5-point live test suite in `wolfram/src/main.rs` validating job hierarchy, handle isolation, dual-stack allocation, context switching, and ready queue operations.
- **Zero-Warning Toolchain Verification:** Maintained clean compilation across both `riscv64gc-unknown-none-elf` and `x86_64-unknown-none` targets with zero warnings under `-D warnings`.
