<p align="center">
  <img src="docs/assets/ferber.png" width="180" alt="Ferber logo">
</p>

<h1 align="center">Ferber</h1>

<p align="center">
  <em>A capability-based microkernel. Zero ambient authority. Programs earn every resource they touch.</em>
</p>

<p align="center">
  <a href="https://github.com/ferber-os/ferber/actions/workflows/ci.yml"><img src="https://github.com/ferber-os/ferber/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/ferber-os/ferber/releases"><img src="https://img.shields.io/github/v/release/ferber-os/ferber?include_prereleases&color=blue" alt="Release"></a>
  <a href="https://github.com/ferber-os/ferber/stargazers"><img src="https://img.shields.io/github/stars/ferber-os/ferber?style=social" alt="Stars"></a>
  <a href="https://github.com/ferber-os/ferber/network/members"><img src="https://img.shields.io/github/forks/ferber-os/ferber?style=social" alt="Forks"></a>
  <a href="https://github.com/ferber-os/ferber/issues"><img src="https://img.shields.io/github/issues/ferber-os/ferber" alt="Issues"></a>
  <a href="https://github.com/ferber-os/ferber/pulls"><img src="https://img.shields.io/github/issues-pr/ferber-os/ferber" alt="Pull Requests"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--2.0--only-blue" alt="License"></a>
</p>

Ferber is an experimental capability-based microkernel written in Rust. Ferrum
is its userspace environment. It runs on RISC-V 64 and x86-64 in QEMU, with
additional physical x86-64 validation.

Processes receive resources explicitly. A handle identifies a kernel object and
grants specific operations on it. New processes start with empty handle tables;
startup installs only the capabilities deliberately delegated to them.

## What runs today

Ferrum init starts a read-only filesystem service, reads a service configuration
from an initramfs archive and launches independently built executables. Processes
communicate over bounded channels, share capabilities through explicit transfer,
and cooperate through yield and blocking waits. The kernel manages sparse RAM,
VMO-backed memory, protected address spaces and hierarchical process Jobs.

The current release is [Uranium-238 v0.3.1](docs/releases/Uranium-238-v0.3.1.md).
The init, filesystem and configured-service implementation has passed physical
x86-64 checks. See the [verification reference](docs/reference/verification.md)
for the tested build and artifact scope.

There is no interactive shell yet. `fer`, the planned system CLI, is not available.
A console-capable shell, framebuffer terminal and `fer` skeleton are the next
[roadmap](ROADMAP.md) milestone, followed by AArch64. Musl compatibility, persistent
storage, preemptive scheduling and SMP are planned. Ferber is not suitable for
production use. Current userspace runs on one active CPU with interrupts disabled.

Ferber was previously called Wolfram.

## Build and boot

Install Rust nightly, the required targets and QEMU as described in
[Running Ferber](docs/guides/running.md). From the repository root:

```sh
make run INIT=1       # boot Ferrum in RISC-V QEMU
make x86-iso INIT=1   # create the x86-64 UEFI ISO
```

A successful init boot ends with `init_waiting: PASS` and remains idle. It does
not display a shell prompt. Use [Running Ferber](docs/guides/running.md) for
requirements, debugging and expected output.

## Read the documentation

Start at the [documentation index](docs/README.md). The
[architecture guide](docs/guides/architecture.md) explains the kernel boundary;
[boot and services](docs/guides/boot-and-services.md) follows a working boot.
Exact interfaces live in the reference pages. Historical release notes describe
what worked at each release, including limitations that have since changed.

## Contribute

Your first contribution does not need to touch the kernel. Documentation,
localization, tests, tooling and diagnostics are useful starting points. Rust,
architecture and security work have their own paths in
[CONTRIBUTING.md](CONTRIBUTING.md).

Prefer issues labeled `good first issue`, `help wanted`, or an equivalent label.
The roadmap describes direction; an unchecked item is not automatically a starter
task. Kernel architecture changes require reading the architecture and capability
references first. Contributions involving voice text or Easter eggs follow the
[style guide](EASTEREGGS.md).

Report capability-security vulnerabilities with an issue titled `[SECURITY]`
and include a detailed reproduction. See the contribution guide for the full
reporting guidance.

## Releases and license

Releases use element names. Uranium identifies the current experimental series;
a release tag records a milestone, not a claim of production readiness. Browse
[release history](docs/releases/README.md) for changes and compatibility notes.

Ferber is licensed under [GPL-2.0-only](LICENSE).

## Contributors

<p align="center">
  <a href="https://github.com/ferber-os/ferber/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=ferber-os/ferber" alt="Contributors" />
  </a>
</p>
