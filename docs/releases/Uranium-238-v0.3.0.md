# Uranium-238 v0.3.0

*Ferrum init now constructs and launches an independently built resident service on RISC-V64 and x86-64. Staged task creation keeps construction authority separate from running-process lifetime, while tracked wake sources distinguish legitimate multi-service waiting from cooperative deadlock.*

## Userspace constructs a child before publishing it

Task ABI v1 adds syscalls 17–22: process create, map, give, initial-thread create,
start and abort. Creating a process requires explicit MANAGE authority through a
live concrete Job capability. Controllers revalidate the creating authority and
its ancestry before construction operations. Abort remains available after Job
authority revocation when the controller itself is still live.

A draft begins with zero handles and cannot execute before successful start.
Mapping accepts bounded owned-RAM VMOs, retains backing independently of handles,
requires sealed backing for RX and retains writable permits for RW/NX. Guarded
stack ranges remain reserved. Capability delegation moves eligible VMO, leaf
Resource or channel capabilities into actual assigned child slots. It does not
implicitly clone parent authority or enable graph-owning channel payloads.

Initial-thread creation prepares one guarded user stack, a kernel-stack VMO and
startup ABI v1. Scheduler admission reserves fallible storage before publishing
one runnable task. Failed preparation remains retryable; final controller release
aborts an unstarted draft. Started controllers retain weak process references;
closing them cannot kill a started child. Job and scheduler ownership continues
until termination, with final reclamation on the dispatcher stack. Admission is
bounded to 64 live processes/tasks and remains subject to hierarchical budgets.

## Init receives executable bytes and explicit launch authority

A bounded boot bundle contains exactly two independently linked static ELF
executables: init and its first service. The allocation-free parser rejects
invalid counts, ordering, flags, padding, overlaps, overflow, truncated ranges
and trailing bytes. The existing 4 MiB firmware payload bound and BootInfo v3
remain unchanged. UEFI retains the transport filename `BOOT.ELF`; RISC-V uses
initrd delivery. This format is not a general initramfs or authenticated boot.

The kernel loads init and delegates a weak Job authority for a dedicated 16 MiB
services Job plus a sealed read-only source-image VMO through bootstrap protocol
v2. Ferrum uses the shared bounded ELF parser, creates fresh segment VMOs,
populates file bytes and zero-filled BSS, seals executable backing, installs child
mappings, moves a bootstrap endpoint, creates the initial thread and starts it.
Temporary source mappings and construction handles are released after setup.
The kernel does not construct the running child on init's behalf.

## Readiness requires execution and a rooted wait graph

The service announces READY and answers a PING forwarded through init. Kernel
evidence observes successful sends from the actual service process, verifies the
paired endpoint and pending challenge, and checks that the same service remains
alive and blocked on that endpoint before reporting success. Unsolicited or
replayed PONGs cannot satisfy a later challenge. Weak observation references do
not keep channels alive. Unexpected child exits receive explicit diagnostics.

The scheduler connects blocked readers to admitted tasks holding live WRITE
capabilities and requires the bounded wait graph to reach a registered source.
Source leases use weak endpoint identity and checked generations. Holding both
ends of a closed cycle does not make it a legitimate wake source. The normal
bootstrap source permits a validated terminal steady state with init and its
service blocked. Interrupts remain masked there.

A separate `service-wake-probe` queues concrete software work after quiescence,
resumes both processes, checks challenge 75 and releases its source lease.
This demonstrates cooperative software delivery, not hardware interrupt wakeup.

Finite warmup and measured shutdown runs precede the resident run. The service
exits zero, init exits with its expected closure status one, and dispatcher
reclamation restores frame, capability-node and process baselines.

## Build names match the program they select

The Cargo feature is now `init`, replacing `minimal-init`. Make uses `INIT=1`.
Cargo cfgs, scripts, CI and active documentation use the new name; historical
release notes retain the terminology used by their releases. `service-wake-probe`
depends on `init`. The feature rename does not alter the syscall ABI.

`make run INIT=1` builds and boots the bundle on RISC-V.
`make x86-iso INIT=1` packages the release init/service ISO. CI publishes that
configuration. The shared ELF parser is used by both kernel loading and Ferrum's
service construction.

## Verification includes physical x86 service launch

Local host suites and the complete RISC-V/x86 QEMU smoke matrices passed before
the version bump. Coverage includes production task syscalls and lifetime rules,
allocation failures and admission retry, scheduler wait graphs/source leases,
bundle and protocol validation, malformed/replayed bootstrap messages and native
service execution. Existing capability, channel, VMO, paging, accounting and ELF
regressions also passed. Four independent adversarial reviews completed with no
confirmed issues remaining after fixes.

After the feature rename, both architecture configurations built and passed
Clippy with warnings denied. Fresh RISC-V init/replay boots and x86 release init
and software-wake ISO boots passed. Freestanding ELF and log-parser checks passed.
After the version bump, release init Clippy passed on both kernel targets and
the UEFI loader with warnings denied. The ten task tests, four scheduler tests,
bundle/protocol tests and strict log-parser self-check passed again. These local
observations precede hosted Actions validation for this tag.

A physical x86-64 boot on the 32 GiB MSI B650M-A PRO WIFI reached:

```text
[FERBER_TEST] memory_verification: PASS
[FERBER_TEST] boot_payload: PASS
[FERBER_TEST] service_launch: PASS
[FERBER_TEST] multiservice_idle: PASS
[FERBER_TEST] init_ready: PASS
[FERBER_TEST] init_waiting: PASS
```

That run managed 31.46 GiB and used the pre-bump `0.2.14` banner. The dedicated
software-wake build was not flashed. No physical RISC-V result is claimed.

The kernel and UEFI loader advance to `0.3.0` without dependency upgrades.
This remains a prerelease. Execution remains cooperative,
single-CPU and integer-only with interrupts masked. Initramfs, named-service
loading, namespace/filesystem services, general service management, independent
user-fault recovery, preemption, IRQ-safe allocation and SMP remain pending.

See the [task ABI](../task-creation.md), [init contract](../init.md) and
[boot-bundle contract](../boot-bundle.md) for exact rights, bounds and lifetimes.
