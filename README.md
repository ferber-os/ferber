<h1 align="center">Wolfram</h1>
<p align="center">
  <em>A capability-based microkernel. Zero ambient authority. Programs earn every resource they touch.</em>
</p>

<p align="center">
  <a href="https://github.com/notvcto/wolfram-os/actions/workflows/ci.yml"><img src="https://github.com/notvcto/wolfram-os/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/notvcto/wolfram-os/releases"><img src="https://img.shields.io/github/v/release/notvcto/wolfram-os?include_prereleases&color=blue" alt="Release"></a>
  <a href="https://github.com/notvcto/wolfram-os/stargazers"><img src="https://img.shields.io/github/stars/notvcto/wolfram-os?style=social" alt="Stars"></a>
  <a href="https://github.com/notvcto/wolfram-os/network/members"><img src="https://img.shields.io/github/forks/notvcto/wolfram-os?style=social" alt="Forks"></a>
  <a href="https://github.com/notvcto/wolfram-os/issues"><img src="https://img.shields.io/github/issues/notvcto/wolfram-os" alt="Issues"></a>
  <a href="https://github.com/notvcto/wolfram-os/pulls"><img src="https://img.shields.io/github/issues-pr/notvcto/wolfram-os" alt="Pull Requests"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--2.0--only-blue" alt="License"></a>
</p>

<br>

Wolfram is a microkernel operating system written in Rust. RISC-V 64 is its
current QEMU reference port; x86-64 UEFI is the next physical-hardware target.
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
Permission bits, UIDs, ACLs, SELinux policies — all of these are answers to the question
"should this process be allowed to do this?" asked after the process already named the resource.

Wolfram asks a different question: **how did this process get a reference to that resource at all?**

If it doesn't have a handle, it can't name it. If it can't name it, the question never arises.
Capability possession *is* the permission. There is no separate check.

This is called capability-based security. It is not a new idea.
Wolfram is a new implementation of it, in Rust, for modern hardware,
with zero legacy constraints and zero tolerance for ambient authority.

---

## Architecture

```
Wolfram Kernel (microkernel)
├── Capability enforcement     — the core. handle tables, rights, attenuation, revocation.
├── Scheduler                  — jobs → processes → threads. no fork. no ambient inheritance.
├── IPC                        — async channels + FastCall. handle transfer is move semantics.
├── Memory                     — VMO-everything. no anonymous memory. hardware-enforced rights.
└── HAL                        — architecture abstraction. RISC-V and x86-64 first.

Ferrum (userspace foundation)
├── libc port                  — musl-based, Wolfram syscall ABI
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

## What Wolfram Is Not

- Not a Linux distribution
- Not a Linux replacement (yet)
- Not API-compatible with POSIX (by design)
- Not finished
- Not safe to run in production
- Not something you should daily drive today

---

## What Wolfram Will Be

- A kernel you can understand completely
- A security model that isn't bolted on
- A system where `fer cap audit <pid>` tells you everything
- Something worth daily driving
- A community project with a real identity

---

## Versioning

Wolfram uses element names for releases.

| Tier | Elements | Meaning |
|---|---|---|
| Nightly | Uranium, Thorium, Plutonium | Radioactive. Unstable by definition. |
| Unstable | Lithium, Sodium, Potassium | Reactive. Handle with care. |
| RC | Fluorine, Chlorine, Bromine | Getting closer. Still sharp edges. |
| Stable | Helium, Neon, Argon, Krypton, Xenon | Inert. Doesn't react. Ships. |

First release: **Uranium-238-v0.1.0** (nightly). Phase 1 has reached its first
hardware milestone on the MSI B650M-A PRO WIFI; the release tag identifies
that milestone, not a feature-complete or security-complete system.
Current milestone: **Uranium-238-v0.2.1** adds userspace handle closure,
rights-checked duplication, and capability inspection on both RISC-V 64 and
x86-64. It builds on Phase 3 M1 (`v0.2.0`): user-mode entry, returning
syscalls, and supervisor-memory isolation. The normal resident probe passes
on physical x86-64 hardware. Scheduling, ELF loading, and init remain pending.
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
git clone https://github.com/notvcto/wolfram-os
cd wolfram-os
make run     # boots in QEMU
make debug   # boots with GDB server on :1234
```

`make run` executes the in-kernel checks, enters a resident user-mode probe,
returns through yield and unknown syscalls, exercises handle operations, and
exits through the kernel. A successful normal boot prints both
`[WOLFRAM_TEST] privilege_switch_roundtrip: PASS` and
`[WOLFRAM_TEST] handle_syscalls: PASS`, then halts until QEMU is stopped.
The separate `user-isolation-probe` feature attempts a supervisor-memory read
and expects a diagnosed user protection fault followed by
`[WOLFRAM_TEST] privilege_isolation: PASS`.
The separate `user-write-probe` and `user-execute-probe` features require a
user protection fault when writing RX code or entering an NX code page,
respectively. Their markers are `privilege_write_rx: PASS` and
`privilege_execute_nx: PASS`, with the same `[WOLFRAM_TEST]` prefix. Run each
fault probe separately.

The boot allocator reads RAM from the firmware device tree and manages the bank
containing the kernel (up to 4 GiB). It keeps the kernel image, device tree, and
firmware-declared reserved ranges unavailable for allocation. Init is planned
for Phase 3.

The x86-64 kernel and UEFI loader exercise the same user-mode contract in
QEMU/OVMF. `bash scripts/riscv-smoke.sh` and `make x86-smoke` check normal,
supervisor-read, RX-write, and NX-execute boots in debug and release builds.
The x86 roundtrip also checks hostile user direction flags, kernel entry
flags, and sanitized return flags with preserved arithmetic flags. The x86
suite retains its deliberate invalid-opcode exception check.
`bash scripts/x86-iso-smoke.sh` additionally boots a release-profile ISO
through OVMF's CD-ROM path using the shared release packaging script. Its
kernel enables only `qemu-debug-port` diagnostics for automated log capture;
it is not byte-identical to the published kernel. CI runs all three suites.
`scripts/x86-preview.sh`
builds the actual bare-metal kernel and checks the ELF that the loader expects;
see the [UEFI loader notes](boot/uefi/README.md) for build and packaging steps.
The MSI B650M-A PRO WIFI previously booted the Phase 1 path from USB,
initialized the physical allocator, and reached its expected diagnosed panic.
A release-profile build of the current handle-syscall implementation also
completed the normal resident probe on physical x86-64 hardware, reaching both
PASS markers and the expected halt. Dedicated protection-fault builds remain
validated in QEMU.

The resident ABI uses Linux-style argument registers with Wolfram syscall
numbers: `0` is an immediate-return yield hint, `1` exits the resident thread,
`2` closes a handle, `3` duplicates a handle, and `4` inspects its type and
rights. Unknown numbers return `-38`. This is not musl compatibility. The probe
process starts with zero handles, then receives explicitly delegated test
capabilities. See the [syscall ABI](docs/syscall-abi.md) for rights, errors,
and inspection output. Code and both stacks are VMO-backed, and
resident user-copy helpers preflight page permissions before copying through
supervisor physical aliases. See [the memory model](docs/memory.md) for their
caller contract and temporary bootstrap mappings.

The current path runs one active CPU with integer-only resident probes. Interrupt-driven
scheduling, init/ELF loading, concurrent user tasks, and complete exit
reclamation remain future work.

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
  <a href="https://github.com/notvcto/wolfram-os/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=notvcto/wolfram-os" alt="Contributors" />
  </a>
</p>

---

## License

GPL v2. Same as Linux. Derivatives stay open.
