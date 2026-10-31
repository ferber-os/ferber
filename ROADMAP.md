# Ferber Roadmap

This roadmap records implemented milestones and planned work. Checked items
identify completed implementation; they do not imply support beyond the limits
and validation described in the documentation. Unchecked items are plans, not
available interfaces or a queue of starter issues.

The supported architecture paths are RISC-V64 and x86-64. Shared features must
preserve matching behavior on both. AArch64 is planned; it becomes an equal
parity target once the port meets the existing contracts.

---

## Immediate Priority — Functional Ferrum Shell and `fer`

Deliver a usable interactive Ferrum shell and a `fer` command skeleton before
starting AArch64. Use the existing initramfs, filesystem sessions, task authority
and channel waits to connect input, command execution and visible output.

- [ ] Launch the shell through init from independently built initramfs executables, with explicitly delegated console, filesystem and task-creation authority.
- [ ] Provide working console input, line editing, command parsing, useful builtins and clear command/error output; launch an executable by path and return to the prompt after it exits.
- [ ] Provide filesystem navigation and file reads through initfs sessions without introducing global pathname authority.
- [ ] Ship a runnable `fer` skeleton with help/version output and channel-based service requests; document only implemented commands.
- [ ] On x86-64, provide the boot-framebuffer terminal below: Spleen font, configurable foreground/background colors, line wrapping and automatic scrolling, without requiring a GPU driver or compositor.
- [ ] Keep equivalent shell and `fer` behavior available through serial on RISC-V 64; isolate framebuffer-specific rendering behind the console interface.
- [ ] Verify input, command execution, child cleanup, console backpressure and output exceeding screen height in QEMU; validate the x86-64 framebuffer shell on physical hardware.

## Next Priority — AArch64 Port

Start the AArch64 port after the functional shell, `fer` skeleton and x86-64
framebuffer milestone. Establish the third architecture before further broad
service-manager and driver expansion, while the kernel remains manageable.
RISC-V 64 and x86-64 remain supported throughout the port; AArch64 reaches the
same shared contracts before being described as a parity target.
Once the port reaches parity, every new feature and ABI change must ship with
matching behavior and verification on AArch64, RISC-V 64 and x86-64. AArch64
then receives matching implementations and verification for subsequent features.

- [ ] Establish an AArch64 QEMU `virt` reference boot, freestanding Rust build, linker layout, serial diagnostics and repeatable debug/release smoke scripts.
- [ ] Define firmware/boot handoff and exception-level normalization into EL1; park secondary CPUs and retain the current single-active-CPU execution model.
- [ ] Implement RAM discovery and reservations, permanent kernel mappings, RAM aliases and the paging HAL with explicit permissions, translation invalidation and required cache/barrier ordering.
- [ ] Implement exception vectors, trusted kernel-stack entry, integer context switching, EL0 entry/return, sanitized user state and the Ferber syscall register convention.
- [ ] Preserve VMO backing ownership, checked user copies, zero ambient authority, task cleanup, capability semantics and scheduler/wait-generation invariants.
- [ ] Port independently linked Ferrum executables, ELF machine/ABI validation and boot-envelope delivery; establish initramfs, filesystem sessions, configured service startup, rooted idle and software-wake parity.
- [ ] Add protection-fault probes, allocator/paging tests, Clippy checks and CI coverage; require matching shared ABI behavior across all three architectures.
- [ ] Document the boot protocol, supported CPU assumptions and deferred FP/SIMD, interrupt, SMP and hardware-driver work. Track physical AArch64 validation separately from QEMU results.

## Phase 1 — Reliable Boots (Complete)

Phase 1 establishes repeatable, diagnosed boots across emulated and bare-metal targets. The acceptance criterion is reaching an intentional, diagnosed panic without silent firmware resets.

### RISC-V 64 QEMU Reference Port
- [x] OpenSBI supervisor handoff at `0x80200000`
- [x] Early 16550 UART serial driver for boot diagnostics
- [x] Flattened Device Tree (FDT) parsing for RAM discovery
- [x] Early physical bitmap frame allocator (`bitmap.rs`)
- [x] Supervisor trap vector setup (`trap.S`, `trap.rs`)
- [x] Secondary hart parking prior to boot stack initialization
- [x] Multi-hart smoke verification at 128 MiB and 256 MiB (`scripts/riscv-smoke.sh`)

### x86-64 UEFI Reference Port
- [x] Common architecture-neutral boot information protocol (`BootInfo` struct)
- [x] Standalone UEFI bootloader crate (`boot/uefi`)
- [x] UEFI memory map extraction and physical frame reservation
- [x] GOP framebuffer discovery and kernel console rendering
- [x] Identity paging setup and long-mode handoff with EFER.NXE and CR0.WP
- [x] Freestanding position-independent kernel ELF (`ET_DYN`, zero `NEEDED`, `R_X86_64_RELATIVE` only)
- [x] Pre-boot ELF inspection and validation script (`scripts/x86-preview.sh`)
- [x] IDT setup and unexpected exception vectoring (tested via deliberate `#UD` vector 6 probe)
- [x] Automated QEMU/OVMF smoke testing across debug, release, and exception builds (`scripts/x86-smoke.sh`)
- [x] Automated bootable FAT32 ISO image construction (`xorriso`, `dosfstools`, `mtools`)
- [x] Bare-metal validation on real hardware (Kingston USB flash on MSI B650M-A PRO WIFI, 4096 MiB RAM)

*Release Milestone: `Uranium-238-v0.1.0` and `Uranium-238-v0.1.1`.*

---

## Phase 2 — Kernel Core (Complete)

Phase 2 builds the microkernel foundation: dynamic memory allocation, hardware-enforced virtual memory, capability derivation, execution containers, context switching, and IPC primitives.

### Memory & Allocation Subsystems
- [x] Dynamic buddy allocator (`heap.rs`, orders 0..=7: 32 B to 4096 B blocks)
- [x] Contiguous frame allocator bypass for multi-page requests exceeding 4096 B
- [x] Upper-bound memory ceiling (`MAX_ALLOC_SIZE = 128 MiB`) with OOM protection
- [x] Virtual Memory Objects (`vmo.rs`):
  - [x] Dynamic contiguous physical page backing
  - [x] Slice read/write operations with checked arithmetic bounds enforcement
  - [x] Capability rights enforcement (`MAP`, `READ`, `WRITE`, `EXECUTE`)
  - [x] Physical MMIO VMOs for hardware device mapping

### Paging Hardware Abstraction Layer (`AddressSpace`)
- [x] Common hardware-agnostic `AddressSpace` trait (`paging.rs`)
- [x] Strict 1:1 feature parity between RISC-V 64 and x86-64
- [x] **RISC-V 64 Sv39 (`arch/riscv64/paging.rs`)**:
  - [x] 3-level radix page table walk (`VPN[2..0]`)
  - [x] Canonical 39-bit virtual address sign-extension check
  - [x] PTE bitfield encoding (`V, R, W, X, U, G, A, D`) with $W \implies R$ rule compliance
  - [x] 44-bit physical page number positioning (bits 53..10)
  - [x] Dynamic intermediate table frame allocation and zeroing
  - [x] `satp` CSR activation (`MODE=8`) with single-page and global `sfence.vma` TLB shootdowns
  - [x] Leak-free recursive `Drop` table deallocation
- [x] **x86-64 PML4 (`arch/x86_64/paging.rs`)**:
  - [x] 4-level page table walk (`PML4 -> PDPT -> PD -> PT`)
  - [x] Canonical 48-bit address validation guarding against non-canonical gap `#GP(0)`
  - [x] Intermediate table permission propagation (`PTE_PRESENT | PTE_WRITABLE | PTE_USER`)
  - [x] Upper-half kernel space synchronization (entries 256..511 cloned from boot CR3)
  - [x] User-only recursive teardown in `Drop` (strictly freeing entries 0..255, preserving shared kernel tables)
  - [x] CR3 activation with single-page `invlpg` and full shootdown via CR3 reload
- [x] Superpage translation resolution (1 GiB and 2 MiB leaves)

### Capability Primitives & Handle Tables
- [x] 10 core capability rights bitflags (`Rights: u32`)
- [x] Capability nodes (`CapNode`) with wait-free atomic revocation (`valid: AtomicBool`)
- [x] Type-annotated compile-time handle wrappers (`Handle<T, R>`) using zero-sized marker types
- [x] Attenuation invariants: child nodes strictly inherit a subset of parent rights
- [x] Per-process handle tables (`HandleTable`):
  - [x] Zero ambient authority: processes are born with empty handle tables (0 handles)
  - [x] O(1) indexed lookup with liveness and revocation validation
- [x] Capability linear move semantics:
  - [x] `transfer_send`: validates `Rights::TRANSFER` and excises handle from sender immediately upon dispatch
  - [x] `transfer_recv`: installs capability node into recipient handle table
  - [x] Owned in-flight capability references; discarded messages release ownership without revoking surviving aliases

### Task Container Model & Scheduling
- [x] Zero-dependency atomic spinlock (`SpinLock<T>`) with Acquire/Release memory ordering
- [x] Hierarchical Job Tree (`job.rs`):
  - [x] Singleton root job (`Job::create_root()`)
  - [x] Acyclic job hierarchy with weak parent references
  - [x] Memory policy fields (`memory_limit`, `memory_used`)
  - [x] Enforce [hierarchical Job memory charges and limits](docs/job-accounting.md)
  - [x] Recursive subtree destruction (`Job::kill()`): synchronous cascade teardown guaranteeing **zero orphan processes**
- [x] Process Containers (`process.rs`):
  - [x] Isolated `ArchAddressSpace` bound at creation
  - [x] Empty `HandleTable` initialization
  - [x] Thread container tracking and death propagation
- [x] Thread Model (`thread.rs`):
  - [x] Dual-stack execution model: dedicated kernel stack VMO and user stack VMO
  - [x] Thread states (`Ready`, `Running`, `Blocked`, `Dead`)
- [x] Hardware Context Switching:
  - [x] RISC-V 64: 112-byte `CpuContext` (`ra, sp, s0..s11`) assembly save/restore in `context.S`
  - [x] x86-64: 56-byte `CpuContext` (`rsp, rbp, rbx, r12..r15`) assembly save/restore in `context.S` with mandatory `cld` Direction Flag sanitization before `ret`
- [x] Early Cooperative Scheduler (`scheduler/`):
  - [x] Global ready queue backed by O(1) `VecDeque`
  - [x] Lock inversion hygiene: scheduler lock dropped prior to thread mutation
  - [x] `spawn`, `ready_count`, and `sched_yield`

### Inter-Process Communication (IPC)
- [x] **Asynchronous Channels (`channel.rs`)**:
  - [x] Paired bidirectional endpoints (`ChannelEndpoint`) with shared pair state
  - [x] Bounded message queues (default max depth: 64 messages)
  - [x] Message size limits (64 KiB maximum payload)
  - [x] Handle migration limits (64 handles per message)
  - [x] Send backpressure enforcement (`QueueFull` error)
  - [x] Signal state bitmask tracking (`SIGNAL_READABLE`, `SIGNAL_WRITABLE`, `SIGNAL_PEER_CLOSED`)
  - [x] Peer termination notification on endpoint drop
- [x] **FastCall Synchronous IPC (`fastcall.rs`)**:
  - [x] Low-latency register-only IPC (~50-150 cycles) bypassing the scheduler runqueue
  - [x] Thread donation execution model: caller blocks and donates remaining time-slice directly to target
  - [x] Register calling convention: 5 arguments (`FastCallArgs`: `arg0..arg4`), 2 returns (`FastCallRet`: `ret0..ret1`)
  - [x] Zero memory allocation and zero cache line churn

*Release Milestones: `Uranium-238-v0.1.2` through `Uranium-238-v0.1.7`.*

---

## Phase 3 — Userspace (In Progress)

Phase 3 transitions execution from kernel-only testing into unprivileged userspace, implementing the system call boundary, userspace privilege switching, the initial user process, and foundational runtime services.

### Physical Memory Discovery & Allocation
- [x] Discover and make allocatable all usable firmware-reported RAM within architecture/alias limits, including discontiguous ranges, while preserving reserved ranges on RISC-V FDT and x86-64 UEFI.
- [x] Permanent high-half kernel mappings and packed RAM aliases on both architectures; private user lower halves and shared supervisor upper halves.
- [x] RAM-size-independent boot verification of detected extents, permanent aliases, owned-page allocation and user copies, and transactional allocator accounting.
- [ ] Reclaim UEFI boot-services memory after loader ownership and references end.
- [ ] Define IRQ-safe allocator locking and SMP initialization/publication before enabling allocation from interrupt context or multiple CPUs.

### Privilege Boundary & System Call ABI
- [x] Userspace privilege level switching:
  - [x] RISC-V 64: User mode (U-mode) entry via `sret`, configuring `sstatus.SPP = 0`, `sstatus.SPIE = 1`, and `sepc`
  - [x] RISC-V 64: Kernel stack swap via `sscratch`, saved user registers, and nested supervisor trap preservation
  - [x] x86-64: Ring 3 entry and syscall return via `iretq`
  - [x] x86-64: GDT configuration for Ring 3 code/data segments (`USER_CS`, `USER_DS`)
  - [x] x86-64: TSS configuration with `rsp0` kernel stack pointer
  - [x] x86-64: `MSR_LSTAR`, `MSR_STAR`, `MSR_FMASK`, and EFER.SCE setup for `syscall`
  - [x] x86-64: Trusted syscall stack selected through `swapgs` before using user-controlled RSP
- [x] Resident user probes on both architectures: returning yield/unknown syscalls, register/stack checks, exit, and separate supervisor-read, RX-write, and NX-execute fault tests
- [ ] System call dispatch table:
  - [x] Resident dispatcher: Ferber numbers, six argument registers, and `-38` for unknown numbers, independent of probe bookkeeping
  - [x] Bootstrap task operations: immediate-return yield hint (`0`) and thread exit (`1`) followed by halt
  - [x] Capability metadata inspection: `sys_cap_inspect` requires `INSPECT` and copies type/rights to validated user memory
  - [x] Capability mutation: `sys_cap_attenuate` with `DUPLICATE`, validated rights subsets, and owned child nodes; `sys_cap_revoke` with `MANAGE` and descendant invalidation
  - [x] Handle operations: `sys_handle_close`, `sys_handle_duplicate` with `DUPLICATE` enforcement and shared-node aliases
  - [x] Nonblocking channel IPC: `sys_chan_create`, `sys_chan_send`, `sys_chan_recv`; validated user buffers, transactional VMO/leaf Resource capability moves, bounded queues, and peer closure
  - [ ] Ownership-cycle policy for transferring channel endpoints and other graph-owning capabilities
  - [x] Resident channel wait-queue integration: blocking receive (`10`) requires `READ | WAIT`, parks threads, and wakes on message arrival, final peer closure, or authority cancellation; nonblocking receive (`9`) remains unchanged
  - [ ] FastCall IPC: `sys_fastcall` register transfer
  - [ ] VMO operations:
    - [x] Bounded owned-RAM `sys_vmo_create`, `sys_vmo_read`, `sys_vmo_write`; synchronized staged byte copies and object-lifetime user budget
    - [x] RO/RW-NX `sys_vmo_map`, exact-record `sys_vmo_unmap`; explicit MAP creation, shared backing ownership and safe teardown
    - [x] Transactional range installation, empty-table pruning and bounded user mapping/table resources
    - [x] Executable VMO publication/sealing across writable aliases: explicit EXECUTE creation, global writable-mapping permits, irreversible seal, RX mapping and local instruction synchronization on both ports
  - [x] Staged task creation: process create/map/give, one initial thread, explicit start/abort ([ABI](docs/task-creation.md))
  - [x] Cooperative resident yield and thread/last-thread process exit reclamation on a permanent-root dispatcher stack; repeated-run frame/node baselines and empty registry checks
  - [x] Extend cooperative admission and exit reclamation to user-created tasks, bounded to 64 live processes/tasks
- [x] Resident user memory safety validation:
  - [x] Canonical user ranges, checked arithmetic, and effective user/read/write page permissions
  - [x] `copy_from_user` / `copy_to_user`: whole-range preflight and physical-alias copies under an unsafe stable-mapping/nonaliasing caller contract
- [ ] Concurrent unmapping and recoverable user-copy faults

*M1 (`Uranium-238-v0.2.0`) retains one bootstrap process and its VMO backing until
halt. Secondary harts stay parked; interrupts, FP/vector user state, and
preemptive scheduling are outside this milestone. Linux-style registers use
custom Ferber syscall numbers and do not provide musl compatibility.*

The resident handle-syscall path (`Uranium-238-v0.2.1`) extends M1 with
explicitly delegated test capabilities, inspection, same-node duplication, and
independent handle closure on both architectures. It retains the single-task
and halt-on-exit limits.

The resident mutation path (`Uranium-238-v0.2.2`) adds attenuation and descendant
revocation, reference-owned nodes/backing, and a 1,024-node live/reserved quota. Normal probes require the
additional `capability_mutation: PASS` marker. This extends the resident path;
it does not add scheduling, init, VMO-mapping revocation, or complete exit cleanup.
See [the syscall ABI](docs/syscall-abi.md).

The cooperative resident path (`Uranium-238-v0.2.5`) runs isolated tasks on one
active CPU with interrupts disabled. Yield switches ready tasks; blocking channel receive parks
until message arrival, peer closure, or cancellation. Final thread/process
resources are released outside locks after switching to the permanent-root
VMO-backed dispatcher stack. Normal boots additionally require
`cooperative_scheduling: PASS`, `blocking_channel_receive: PASS`, and
`task_reclamation: PASS`. Graph-owning capability transfer remains rejected;
its lifetime policy is still open. This path does not add ELF/init, userspace
task creation, VMO syscalls, FastCall syscalls, preemption, or SMP.

### Userspace Binary Loading & Execution
- [x] Bounded static ELF loader for Checkpoints A and B (`shared/elf.rs`, `ferber/src/kernel/elf_load.rs`):
  - [x] Bounded static ELF64 `ET_EXEC` parsing with architecture, range, alignment, entry, resource-limit, and unsupported-feature validation
  - [x] VMO allocation for ELF segments (`PT_LOAD` headers), initialized file bytes, and zero-filled BSS/page tails
  - [x] Memory mapping with strict permissions: `PT_LOAD` `.text` mapped R-X, `.rodata` mapped R--, `.data`/`.bss` mapped RW-
  - [x] Checkpoint A: separately compiled RISC-V/x86-64 ELF fixture, kernel-constructed zero-handle process, guarded stack, syscall return/yield/exit, and repeated reclamation checks
  - [x] VMO-backed guarded user stack with aligned initial SP
  - [ ] Populate the stack with an auxiliary vector (`auxv`), arguments (`argc`/`argv`), and environment
- [x] Checkpoint B test ABI: explicitly delegate a bootstrap channel to the ELF fixture and verify a request/reply exchange
- [x] Startup ABI v1: bounded record at initial SP, slot-independent bootstrap discovery, and shared integer-only `no_std` Rust entry on both architectures
- [x] Bounded external raw ELF delivery: BootInfo v3, UEFI `BOOT.ELF`, RISC-V initrd discovery, reserved RO/NX source and boot-lifetime backing
- [x] Integrate startup ABI v1 into minimal Ferrum init and define its versioned bootstrap readiness protocol
- [ ] Init Process (`init`):
  - [x] Root userspace process spawned under root `Job`
  - [x] Explicit bootstrap channel to the kernel-owned startup session; validated terminal waiting
  - [x] Bounded kernel control pump, registered producer generations and rooted multi-service terminal quiescence
  - [ ] Hardware event-driven idle and general kernel control service
  - [x] Ferrum ELF construction and first independently built resident service
  - [ ] Spawns root namespace and additional system servers

Checkpoint A is an opt-in `userspace-elf-probe` build released in
`Uranium-238-v0.2.6`. That release embedded independently linked bytes. The
current path consumes one externally delivered raw ELF through BootInfo v3;
this is not a multi-file initramfs. Its 16 KiB stack has an unmapped guard page
below it. Startup ABI v1
places a versioned 32-byte record at initial SP; the remaining stack is zeroed.
The process is born with an empty handle table, then the kernel explicitly
installs a bootstrap channel with `READ | WRITE` rights before admission and
records its assigned slot. Shared Rust code discovers that handle instead of
assuming slot 0. The kernel
prequeues a challenge, and the ELF fixture uses nonblocking channel syscalls
to receive it and send a reply for the kernel to verify. No service task runs;
this fixture exchange is separate from init's bootstrap protocol. Ferrum init
uses the same startup ABI with a versioned readiness protocol and validated
terminal waiting. Init now constructs its first resident service through task
and VMO syscalls; namespace and additional system servers remain pending. The
earlier Checkpoint A fixture remains kernel-created and needs neither syscall
family. See
[the ELF loading contract](docs/elf-loading.md) and
[startup ABI v1](docs/userspace-startup.md), and
[the boot-payload contract](docs/boot-payload.md).

### Boot Framebuffer Terminal
- [ ] Software terminal over the boot-provided framebuffer, available before a display driver or compositor
  - [ ] Line wrapping and automatic vertical scrolling so boot and runtime logs remain visible after the screen fills
  - [ ] Configurable foreground and background colors, with consistent colors for diagnostic severity
  - [ ] Replace the current font with [Spleen](https://github.com/fcambus/spleen), preserving its BSD 2-Clause license and copyright notice; choose a native bitmap size after framebuffer previews
  - [ ] Respect framebuffer dimensions, stride and pixel format; keep rendering and scroll operations within mapped bounds
  - [ ] Preserve serial diagnostics on both architectures, including systems without a boot framebuffer

### C Runtime & Foundations
- [ ] `musl` libc port:
  - [ ] Ferber architecture target definitions for `musl`
  - [ ] System call stub mapping POSIX calls (`read`, `write`, `mmap`, `exit`) to Ferber capability IPC
  - [ ] Dynamic allocator integration (`malloc` / `free` backed by `sys_vmo_create` / `sys_vmo_map`)
- [ ] Virtual Filesystem (VFS) Service:
  - [ ] Standalone userspace microkernel server
  - [ ] Files and directories represented strictly as capability handles
  - [ ] Mount point namespace isolation per process container
- [ ] Initial ramdisk (`initramfs`):
  - [x] Independently versioned read-only indexed archive and deterministic pack/list/verify/extract tooling
  - [x] Boot envelope delivers init separately from archive; Ferrum launches an archive filesystem service
  - [x] Streaming OPEN/READ/LIST/CLOSE/STAT over explicitly delegated sessions with session-local IDs
  - [x] Bounded channel wait sets and required multi-service startup from `etc/services.conf`
  - [ ] Package the interactive shell and `fer` when available
  - [ ] Integrate the boot filesystem with the general VFS service and process namespaces

### User Shell & Command-Line Skeleton
- [ ] Functional interactive shell over capability-delegated console services: RISC-V serial and x86-64 framebuffer, with serial diagnostics retained
- [ ] `fer` CLI skeleton:
  - [ ] Basic command parser and argument dispatch
  - [ ] Channel IPC communication with init and kernel inspection interfaces

---

## Phase 4 — Drivers

Phase 4 moves device drivers into userspace microkernel services with fine-grained capability isolation.

### Userspace Driver Architecture
- [ ] Driver framework protocol:
  - [ ] Userspace driver process container isolation (no kernel-mode driver code)
  - [ ] Hardware MMIO access granted strictly through `Physical VMO` capability handles
  - [ ] Direct Memory Access (DMA) buffer management backed by contiguous pinned VMOs
  - [ ] Hardware interrupt delivery via FastCall signal notifications
- [ ] Hardware timer interrupts & preemption:
  - [ ] RISC-V: SBI timer extension (`sbi_set_timer`) and supervisor timer interrupts (`STIP`)
  - [ ] x86-64: APIC timer setup and periodic/deadline interrupt handling
  - [ ] Preemptive thread scheduling slice ticks

### Device Drivers
- [ ] Serial console driver (16550 UART / ARM PL011 / x86 COM ports)
- [ ] VirtIO network interface driver (`virtio-net` in QEMU):
  - [ ] Virtqueue descriptor ring management
  - [ ] Packet transmit and receive buffers backed by pinned VMOs
- [ ] VirtIO block device driver (`virtio-blk` in QEMU):
  - [ ] Block read/write request processing

### Persistent Storage & First On-Disk Filesystem
- [ ] Capability-controlled block-device service shared by both architectures
- [ ] First on-disk filesystem implemented as a userspace VFS server:
  - [ ] Select and document the initial disk format and validate untrusted on-disk metadata
  - [ ] Read-only mounting, directory traversal, and file reads
  - [ ] Writable files, directory updates, and block allocation with explicit flush and crash-consistency guarantees
  - [ ] Corrupt-image, interrupted-write, and reboot-persistence tests on both architectures

---

## Phase 5 — `fer` Complete

Phase 5 completes the unified Ferber management tool (`fer`), providing end-to-end operational visibility and administrative control over capabilities, processes, memory, and devices.

- [ ] `fer pkg`: Capability-verified package and service management
- [ ] `fer drv`: Driver lifecycle, device attachment, and bus inspection
- [ ] `fer cap`: Capability graph visualizer, lineage audits, attenuation verification, and instant revocation
- [ ] `fer mem`: VMO utilization, address space breakdown, and physical frame accounting
- [ ] `fer ipc`: Channel queue depth, throughput metrics, and wait queue inspection
- [ ] `fer jobs`: Hierarchical job tree visualization and subtree resource utilization

---

## Phase 6 — Daily Driver & Beyond

The long-term roadmap toward running Ferber as a daily operating system.

- [ ] Self-hosting Rust toolchain compiling the Ferber kernel on Ferber OS
- [ ] Preemptive multi-core SMP scheduling with per-CPU runqueues and work stealing
- [ ] Native NVMe block driver using the established block-device service
- [ ] Capability-native on-disk filesystem building on the existing VFS and persistence interfaces
- [ ] Native Ethernet / Wi-Fi network stack (TCP/UDP/IP)
- [ ] Userspace graphical compositor running over GOP framebuffer VMO
- [ ] Hardware IOMMU protection for zero-trust userspace DMA isolation

---

The shell milestone establishes an interactive system. Persistent storage,
preemption, SMP and broad device support remain separate milestones; no date is
promised for daily use.
