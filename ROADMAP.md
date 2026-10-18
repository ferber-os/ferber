# 🗺️ Ferber OS Roadmap

Ferber is built from first principles around zero ambient authority: programs earn every resource they touch. Development is structured in sequential phases, maintaining strict 1:1 architectural parity across RISC-V 64 (`riscv64gc-unknown-none-elf`) and x86-64 (`x86_64-unknown-none` / UEFI).

---

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
  - [x] Memory quota accounting (`memory_limit`, `memory_used`)
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

## Phase 3 — Userspace (Active Next Phase)

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
  - [ ] VMO operations: `sys_vmo_create`, `sys_vmo_read`, `sys_vmo_write`, `sys_vmo_map`, `sys_vmo_unmap`
  - [ ] Task creation: `sys_process_create`, `sys_thread_create`
  - [x] Cooperative resident yield and thread/last-thread process exit reclamation on a permanent-root dispatcher stack; repeated-run frame/node baselines and empty registry checks
  - [ ] Extend scheduling and exit reclamation to user-created tasks
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
- [ ] Userspace ELF loader (`elf_loader.rs`):
  - [ ] 64-bit ELF executable parsing (`ELF64_HDR`, `Phdr`)
  - [ ] VMO allocation for ELF segments (`LOAD` headers)
  - [ ] Memory mapping with strict permissions: `PT_LOAD` `.text` mapped R-X, `.rodata` mapped R--, `.data`/`.bss` mapped RW-
  - [ ] Stack VMO allocation and user stack initialization with auxiliary vector (`auxv`), arguments (`argc`/`argv`), and environment
- [ ] Init Process (`init`):
  - [ ] Root userspace process spawned under root `Job`
  - [ ] Injected initial bootstrap channel handle pointing to kernel control service
  - [ ] Spawns root namespace and system servers

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
  - [ ] Read-only boot image containing userspace binaries (`init`, `sh`, `fer`)
  - [ ] Kernel loads `init` from the image; a userspace filesystem server exposes its contents through the VFS

### User Shell & Command-Line Skeleton
- [ ] Basic interactive shell running over serial console
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

*Daily driver milestone: somewhere past Phase 5. It will take years. That's fine.*
