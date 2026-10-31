# Uranium-238 v0.1.1

*Wolfram gains the capability types and allocation machinery needed by later kernel objects. This establishes the internal object model; process management and a shell are still absent.*

## What changed

- **Type-Safe Capabilities:** Overhauled `capabilities/mod.rs` with `ObjectType`, the `KernelObject` trait, and type-safe `Handle<T: KernelObject, R: RightMarker>` to enforce capability permutations at compile time.
- **Node & Allocator:** Implemented `CapNode` with `NonNull<()>` object references and a working `CapNodeAllocator` that initializes cleanly at boot.
- **Syscall Boundaries:** Added the x86-64 `SyscallFrame` and wired up the x86-64 syscall dispatch stub.
- **IPC & Memory Documentation:** Created `docs/ipc.md` and `docs/memory.md` detailing the intended Phase 2 architecture for buddy allocation and MMU enforcement.
- **Cleanup:** Scrubbed the legacy `W —` signoff pattern from panic handlers and boot messages, officially closing out the Phase 1 narrative.

This release does not yet implement process management or a usable shell; it provides the internal kernel invariants required before VMOs or Tasks can be allocated.
