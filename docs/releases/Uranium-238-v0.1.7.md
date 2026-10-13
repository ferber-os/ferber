# Uranium-238 v0.1.7

This release completes Phase 2 (Kernel Core) at 100% roadmap completion: introducing the FastCall synchronous IPC subsystem with thread donation, register-only parameter transfer, and latency-optimized capability hot paths across RISC-V 64 and x86-64.

## Included

- **FastCall Synchronous IPC Subsystem:** Implemented register-only synchronous IPC (`fastcall()` in `wolfram/src/kernel/ipc/fastcall.rs`) delivering ~50-150 cycle execution for capability hot paths and interrupt dispatch without scheduler queue latency.
- **Thread Donation Execution Model:** Implemented synchronous slice donation where caller transitions directly to `ThreadState::Blocked` while donating its remaining execution slice to the target (`ThreadState::Running`), returning register results synchronously upon completion.
- **Register Argument Protocol:** Defined `FastCallArgs` passing 5 register words (`arg0..arg4`) and `FastCallRet` receiving 2 register words (`ret0..ret1`), avoiding memory allocation and cache line churn.
- **Phase 2 (Kernel Core) 100% Completion:** Concluded all Phase 2 architectural milestones—physical memory allocation, VMO subsystem, Sv39/PML4 paging HAL, capability trees and handle tables, process/thread container model, hierarchical job trees, async channels, and FastCall IPC.
- **Live In-Kernel Verification Suite:** Extended the boot verification suite in `wolfram/src/main.rs` to a 4-point IPC validation battery asserting channel messaging, capability move semantics, peer closure transitions, and cross-process FastCall register donation.
- **Toolchain Invariants & Parity:** Preserved strict 1:1 architecture parity and zero-warning compilation under `-D warnings` across both `riscv64gc-unknown-none-elf` and `x86_64-unknown-none`.
