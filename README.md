<h1 align="center">Ferber</h1>
<p align="center">
  <sub>Ferber was previously called Wolfram.</sub>
</p>

<p align="center">
  <em>A capability-based microkernel. Zero ambient authority. Programs earn every resource they touch.</em>
</p>

<p align="center">
  <a href="https://github.com/notvcto/ferber/actions/workflows/ci.yml"><img src="https://github.com/notvcto/ferber/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/notvcto/ferber/releases"><img src="https://img.shields.io/github/v/release/notvcto/ferber?include_prereleases&color=blue" alt="Release"></a>
  <a href="https://github.com/notvcto/ferber/stargazers"><img src="https://img.shields.io/github/stars/notvcto/ferber?style=social" alt="Stars"></a>
  <a href="https://github.com/notvcto/ferber/network/members"><img src="https://img.shields.io/github/forks/notvcto/ferber?style=social" alt="Forks"></a>
  <a href="https://github.com/notvcto/ferber/issues"><img src="https://img.shields.io/github/issues/notvcto/ferber" alt="Issues"></a>
  <a href="https://github.com/notvcto/ferber/pulls"><img src="https://img.shields.io/github/issues-pr/notvcto/ferber" alt="Pull Requests"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--2.0--only-blue" alt="License"></a>
</p>

<br>

Ferber is a microkernel operating system written in Rust. It boots in QEMU on
RISC-V 64 and x86-64, and has passed resident userspace checks on physical
x86-64 hardware.
It is not a Linux distribution. It is not a fork. It is a kernel built from first principles around
one idea: **programs should not have access to anything they weren't explicitly given.**

There is no root. There is no `sudo`. There is no ambient authority.
Every resource is a capability handle. Every handle has explicit rights.
Every transfer is intentional. Every revocation is instant.

This is not a weekend project. It is not finished. It may never be finished.
But the architecture is right, and that matters more than being finished.

---

## The Model

In most operating systems, security is a layer on top of the kernel.
Permission bits, UIDs, ACLs, SELinux policies. All of these are answers to the question
"should this process be allowed to do this?" asked after the process already named the resource.

Ferber asks a different question: **how did this process get a reference to that resource at all?**

If it doesn't have a handle, it can't name it. If it can't name it, the question never arises.
Capability possession *is* the permission. There is no separate check.

This is called capability-based security. It is not a new idea.
Ferber is a new implementation of it, in Rust, for modern hardware,
with zero legacy constraints and zero tolerance for ambient authority.

---

## Architecture

```
Ferber Kernel (microkernel)
├── Capability enforcement     — the core. handle tables, rights, attenuation, revocation.
├── Scheduler                  — jobs → processes → threads. no fork. no ambient inheritance.
├── IPC                        — async channels + FastCall. handle transfer is move semantics.
├── Memory                     — VMO-everything. no anonymous memory. hardware-enforced rights.
└── HAL                        — architecture abstraction. RISC-V and x86-64 first.

Ferrum (userspace foundation)
├── libc port                  — musl-based, Ferber syscall ABI
├── VFS server                 — filesystem abstraction as a capability service
├── Driver framework           — userspace drivers, MMIO via Physical VMO
└── Init                       — job tree root. everything descends from here.

fer (unified system CLI)
├── fer pkg                    — package management
├── fer drv                    — driver management
├── fer cap                    — capability inspection and revocation
├── fer jobs                   — job tree visualization
├── fer mem                    — memory and VMO inspection
└── fer ipc                    — IPC channel monitoring
```

---

## Capability Model

Every kernel object — process, thread, VMO, channel, device — has handles.
Handles are unforgeable. You cannot guess one. You cannot forge one.
The kernel is the only entity that creates them.

Every handle carries a rights bitmask:

```
DUPLICATE   TRANSFER   READ   WRITE   EXECUTE
MAP         SIGNAL     WAIT   INSPECT MANAGE
```

When you pass a handle to another process, you can only pass equal or fewer rights.
Authority flows downward. It can never be amplified.
A child cannot have more access than its parent granted.
A parent cannot grant more than it holds.

Revocation is instant. Capability nodes are invalidated at the kernel level.
Every handle pointing through a revoked node loses access immediately.

In Rust, this enforced at compile time too:

```rust
// these are different types — you cannot mix them up
Handle<Vmo, Read>
Handle<Vmo, ReadWrite>
Handle<Channel, ReadWrite>

// this is a compile error, not a runtime error
fn write(h: Handle<Vmo, ReadWrite>, data: &[u8]) { ... }
write(read_only_handle, data) // ← compiler rejects this
```

Two layers of enforcement. The compiler catches the obvious mistakes.
The kernel catches everything else.

---

## What Ferber Is Not

- Not a Linux distribution
- Not a Linux replacement (yet)
- Not API-compatible with POSIX (by design)
- Not finished
- Not safe to run in production
- Not something you should daily drive today

---

## What Ferber Will Be

- A kernel you can understand completely
- A security model that isn't bolted on
- A system where `fer cap audit <pid>` tells you everything
- Something worth daily driving
- A community project with a real identity

---

## Versioning

Ferber uses element names for releases.

| Tier | Elements | Meaning |
|---|---|---|
| Nightly | Uranium, Thorium, Plutonium | Radioactive. Unstable by definition. |
| Unstable | Lithium, Sodium, Potassium | Reactive. Handle with care. |
| RC | Fluorine, Chlorine, Bromine | Getting closer. Still sharp edges. |
| Stable | Helium, Neon, Argon, Krypton, Xenon | Inert. Doesn't react. Ships. |

First release: **Uranium-238-v0.1.0** (nightly). Phase 1 has reached its first
hardware milestone on the MSI B650M-A PRO WIFI; the release tag identifies
that milestone, not a feature-complete or security-complete system.
Current milestone: **Uranium-238-v0.2.5** adds cooperative resident scheduling,
blocking channel receive, and resident thread/process reclamation on RISC-V 64
and x86-64. All normal verification markers and the final halt on
physical x86-64 hardware with 32 GiB installed. Execution remains
single-CPU and integer-only, with interrupts disabled. ELF loading, init,
preemption, and userspace task creation remain pending.
Phase 2 concluded with `v0.1.7`.

---

## Roadmap
See [ROADMAP.md](ROADMAP.md).

---

## Building the RISC-V reference port

Requirements:
- Rust nightly (we use features that aren't stable yet — appropriate for a kernel)
- RISC-V target: `rustup target add riscv64gc-unknown-none-elf`
- QEMU: `qemu-system-riscv64`
- A tolerance for triple faults

```bash
git clone https://github.com/notvcto/ferber
cd ferber
make run     # boots in QEMU
make debug   # boots with GDB server on :1234
```

`make run` executes the in-kernel checks and resident user-mode probes. It checks
returning syscalls, handle operations, capability mutation, transactional channel
IPC, cooperative scheduling between isolated tasks, blocking receive, and exit
reclamation. A successful normal boot requires `memory_verification: PASS` and
seven resident markers: `privilege_switch_roundtrip: PASS`, `handle_syscalls: PASS`,
`capability_mutation: PASS`, `channel_syscalls: PASS`, `cooperative_scheduling: PASS`,
`blocking_channel_receive: PASS`, and `task_reclamation: PASS`, each with the
`[FERBER_TEST]` prefix. After all tasks exit and reclamation checks finish, it
halts until QEMU is stopped.
The separate `user-isolation-probe` feature attempts a supervisor-memory read
and expects a diagnosed user protection fault followed by
`[FERBER_TEST] privilege_isolation: PASS`.
The separate `user-write-probe` and `user-execute-probe` features require a
user protection fault when writing RX code or entering an NX code page,
respectively. Their markers are `privilege_write_rx: PASS` and
`privilege_execute_nx: PASS`, with the same `[FERBER_TEST]` prefix. Run each
fault probe separately.

The physical allocator uses all eligible firmware-reported RAM ranges from
the RISC-V device tree or x86-64 UEFI map, including discontiguous ranges.
Reserved ranges, the kernel image, boot data, and allocator metadata remain
unavailable for allocation. Both ports run the kernel in the
high half with permanent supervisor mappings and packed RAM aliases. Sv39's
RAM-alias window is 248 GiB; x86-64 uses four-level paging and the CPU's
physical-address width. Unsupported capacity is rejected rather than
truncated. The heap still limits one allocation to 128 MiB. See
[the memory model](docs/memory.md) for exact boundaries. Init remains planned
for Phase 3.

The x86-64 kernel and UEFI loader exercise the same user-mode contract in
QEMU/OVMF. `bash scripts/riscv-smoke.sh` and `make x86-smoke` check normal,
supervisor-read, RX-write, and NX-execute boots in debug and release builds.
The x86 roundtrip also checks hostile user direction flags, kernel entry
flags, and sanitized return flags with preserved arithmetic flags. The x86
suite retains its deliberate invalid-opcode exception check. Each architecture
also runs a debug `cooperative-deadlock-probe` build where both tasks wait for
messages without a runnable sender. It requires the exact diagnosed panic
`cooperative deadlock: live tasks with no runnable producer` and forbids the
three cooperative success markers. A timeout or an unrelated panic is not a
passing deadlock test. The suites contain 11 RISC-V and 10 x86 cases, plus the
separate release ISO boot. Every ordinary
boot checks the discovered RAM extents, permanent mappings, owned-page
allocation and user copies, and accounting restoration without a RAM-size
threshold or a separate capacity build.
`bash scripts/x86-iso-smoke.sh` additionally boots a release-profile ISO
through OVMF's CD-ROM path using the shared release packaging script. Its
kernel enables only `qemu-debug-port` diagnostics for automated log capture;
it is not byte-identical to the published kernel. CI runs all three suites and
the host channel concurrency/allocation-failure regressions (`bash scripts/channel-tests.sh`).
`scripts/x86-preview.sh`
builds the actual bare-metal kernel and checks the ELF that the loader expects;
see the [UEFI loader notes](boot/uefi/README.md) for build and packaging steps.
The MSI B650M-A PRO WIFI previously booted the Phase 1 path from USB,
initialized the physical allocator, and reached its expected diagnosed panic.
The user booted a release-profile ISO of the v0.2.2 capability-mutation
implementation on physical x86-64 hardware, reaching all three normal PASS
markers and the expected halt. That run preceded the version metadata bump.
Dedicated protection-fault builds remain validated in QEMU.

The resident ABI uses Linux-style argument registers with Ferber syscall
numbers: `0` yields cooperatively, `1` exits the calling thread while surviving
tasks continue, `2` closes a handle, `3` duplicates a handle, `4` inspects its type and
rights, `5` derives a capability with equal or fewer rights, and `6` revokes a
capability and its descendants. Calls `7`–`9` create channels and send/receive
messages without blocking. Call `10` receives with blocking and requires both
`READ` and `WAIT`; channel creation grants `WAIT`. Closing a suspended receive's
source slot cancels it, and revocation cancels waits through that node or its
ancestors. Attenuation requires `DUPLICATE`; revocation
requires `MANAGE`. Unknown numbers return `-38`. This is not musl compatibility.
The probe process starts with zero handles, then receives explicitly delegated test
capabilities. See the [syscall ABI](docs/syscall-abi.md) for rights, errors,
and inspection output. Nodes own their ancestry and object backing; close
releases ownership, and final owners reclaim the nodes and backing. Revocation
does not remove existing VMO mappings. Code and both stacks are VMO-backed;
the RX code VMO spans as many pages as the resident probe needs. Resident
user-copy helpers preflight page permissions before copying through
supervisor physical aliases. See [the memory model](docs/memory.md) for their
caller contract and permanent kernel mappings.

The current path cooperatively schedules integer-only resident tasks on one
active CPU with interrupts disabled. A VMO-backed dispatcher stack on the
permanent kernel root permits safe cleanup after switching away from exited
tasks. Interrupt-driven scheduling, SMP, FP/vector state, init/ELF loading,
userspace task creation, and VMO syscalls remain future work.

---

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the project’s contribution standards. If your work changes kernel architecture, read [docs/architecture.md](docs/architecture.md) and [docs/capability-model.md](docs/capability-model.md) before proposing a design.

### Not sure where to start?

Your first contribution does not need to touch the kernel.

- **New to systems programming:** documentation, tests, CI and tooling, or diagnostics.
- **Comfortable with Rust:** bounded kernel tasks, syscall plumbing, validation, and tests.
- **Comfortable with low-level systems or architecture code:** paging, syscall paths, scheduler, drivers, and RISC-V/x86-64 parity.
- **Security-focused:** capability-model review, unsafe invariants, threat modeling, or fuzzing.
- **Non-code:** translations, documentation, testing, UX, or Easter eggs and voice text where appropriate.

The roadmap describes project direction. Not every unchecked item is a good first issue. Prefer issues labeled `good first issue`, `help wanted`, or an equivalent contributor-friendly label. If you do not see a scoped issue, ask before taking on a large change.

The kernel is written in Rust. The docs are written in English.
Both should be precise, honest, and free of unnecessary complexity.

If you find a security issue in the capability model specifically, that's important. 
Open an issue marked `[SECURITY]` and be detailed.

Non-code contributions are also welcome. 
<br>

## Special thanks to our contributors:

<p align="center">
  <a href="https://github.com/notvcto/ferber/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=notvcto/ferber" alt="Contributors" />
  </a>
</p>

---

## License

GPL v2. Same as Linux. Derivatives stay open.
