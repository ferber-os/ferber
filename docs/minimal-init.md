# Minimal Ferrum init

The `minimal-init` boot mode loads `userspace/init/` as an external static Rust
ELF under the root Job. It uses startup ABI v1 and ordinary channel syscalls.
It announces readiness, then remains alive in blocking receive. It does not
launch services or provide an interrupt-driven event loop.

## Startup delegates one endpoint

The process is born with an empty handle table. Before scheduler admission,
the kernel explicitly installs one bootstrap channel with `READ | WRITE | WAIT`
and records its assigned handle in the existing 32-byte startup record.
Init discovers that handle from the record; its numeric slot is not an ABI.
No job, device, namespace or process-creation authority is delegated.

## The bootstrap exchange is a separate protocol

`shared/init_protocol.rs` defines an exact 16-byte little-endian message:

| Offset | Bytes | Field | Value |
| --- | --- | --- | --- |
| 0 | 8 | Magic | `FRINIT01` |
| 8 | 2 | Version | `1` |
| 10 | 2 | Opcode | `1` START or `2` READY |
| 12 | 4 | Reserved | Zero |

Messages carry zero handles. Wrong lengths, magic, version, opcode, reserved
values or unexpected handles fail startup. The kernel queues START before
admission. Init receives it, validates it, sends READY and calls blocking
receive again on the same handle. A subsequent unsupported message or channel
closure is an error; no services or further commands are defined yet.

## Waiting is an explicit terminal contract

An open channel peer alone is insufficient to justify idle. After ordinary
reaping and wait reconciliation, the kernel accepts terminal waiting only for
the designated init task, after a single valid readiness reply. Init must be
the sole live task and be blocked on its exact bootstrap capability and
channel registration. The capability must remain valid; the pending generation
must be current, unnotified and free of cancellation errors. Unexpected queued
bootstrap traffic also prevents success.

Validation runs on the permanent-root dispatcher stack outside locks. The
kernel retains its peer, init's process, mappings, VMO-backed stacks and pending
receive through terminal halt. This is expected live ownership, not a task
reclamation result. The boot payload source remains reserved independently.

Ordinary scheduler runs retain the exact cooperative-deadlock diagnostic.
A different blocked task, wrong channel, stale registration or revoked source
does not gain an init exemption. Closing or revoking the receive source uses
the existing cancellation and reconciliation path. Unexpected init exit is a
startup failure, including exit status zero.

Interrupts remain disabled. The retained kernel peer establishes protocol
ownership, not an autonomous source of events. Future event-driven idle needs
an interrupt-safe queue check and sleep operation. A non-yielding executable
can still hang the cooperative system; there is no startup deadline.

## Suspended init is exercised before final admission

Each minimal-init boot first runs a private close-after-park check using the
same external executable. The kernel validates readiness and the real pending
receive, closes its peer, and resumes dispatch. Init observes closure and exits
unsuccessfully; the dispatcher reaps it safely. A warmup and measured repetition
check that frame, capability-node and root-job process counts return to their
baselines before the final live instance is admitted. These checks emit no init
success markers and do not add commands to the production bootstrap protocol.

## Build and verification

```sh
make run MINIMAL_INIT=1
make debug MINIMAL_INIT=1
make x86-iso MINIMAL_INIT=1
```

The build produces init separately and transports it as the RISC-V initrd or
UEFI `BOOT.ELF`. Kernel compilation embeds no executable. Select this mode
separately from `USERSPACE_ELF_PROBE=1` and the fault-probe features.

A successful init boot requires exactly these markers:

```text
[FERBER_TEST] memory_verification: PASS
[FERBER_TEST] boot_payload: PASS
[FERBER_TEST] init_ready: PASS
[FERBER_TEST] init_waiting: PASS
```

The last marker requires an actual validated suspended receive. It cannot be
satisfied by ELF loading, sending READY alone or retaining an open peer.
The finite ELF fixture remains a separate regression with its own repeated
execution and reclamation checks. A physical x86-64 boot on the 32 GiB MSI workstation reached all four
markers with the pre-bump `0.2.9` banner. This demonstrates the external init
startup and validated terminal wait, including the prerequisite close-after-park
checks.
