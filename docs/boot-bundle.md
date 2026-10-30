# Init and its first service arrive in one bounded bundle

Init builds consume an externally delivered bundle containing exactly two
independently linked static ELF executables: Ferrum init and its resident test
service. The firmware still transports one opaque file through BootInfo v3.
The bundle is bounded to 4 MiB including metadata and padding. It is not a VFS,
a general archive or an authenticated boot format.

## The parser rejects ambiguous layouts before admission

`shared/boot_bundle.rs` defines the allocation-free parser. All integers are
little-endian. The 32-byte header is followed immediately by two 32-byte entries.

| Header offset | Width | Value |
| --- | --- | --- |
| 0 | 8 | Magic `FRBOOT01` |
| 8 | 4 | Version `1` |
| 12 | 4 | Entry count `2` |
| 16 | 8 | Exact total file length |
| 24 | 8 | Reserved, zero |

| Entry offset | Width | Value |
| --- | --- | --- |
| 0 | 4 | Kind: first `1` (init), then `2` (service) |
| 4 | 4 | Flags, zero |
| 8 | 8 | File offset |
| 16 | 8 | Nonzero exact file length |
| 24 | 8 | Reserved, zero |

Entries must appear in that order with 8-byte-aligned offsets. Payloads cannot
overlap metadata or one another. Padding between entries is zero, and the final
entry ends exactly at the declared bundle length. Unknown/duplicate kinds,
reserved bits, overflow, truncated ranges, oversized input and trailing bytes
are rejected. Payload ELF restrictions are defined by the shared bounded
[ELF parser](elf-loading.md); Ferrum uses that same parser for its service image.

## The kernel delegates bytes and authority, not a running child

The kernel loads only the init executable. It constructs a dedicated services
Job with a 16 MiB allocation limit and grants init a concrete weak Job authority.
The structural Job tree keeps the Job alive; placing its authority in init does
not create a strong Job/process/handle ownership cycle.

The service ELF bytes are copied into a fresh page-rounded owned VMO, zero-filled
past the exact file length, sealed, and granted with READ, MAP and INSPECT rights.
Init can inspect those bytes through an RO mapping. It does not receive WRITE or
EXECUTE authority to that source object. The kernel does not parse and run the
service on init's behalf. Ferrum constructs fresh segment VMOs, writes the
file-backed bytes, retains zero-filled BSS, seals executable segments and installs
the child mappings through the task-construction ABI.

Startup ABI v1 still discovers one bootstrap channel. A separate init protocol
v2 START record supplies exactly two attached handles in order: launch Job, then
service-image VMO. The record carries the exact ELF byte length, independently of
its rounded VMO capacity. Missing/extra handles or malformed records are rejected.
General channel transfer of graph-owning capabilities remains disallowed; these
initial Job grants are an explicit kernel bootstrap operation.

## Ferrum launches, checks and retains a resident service

Init creates a draft child under the supplied Job, maps the validated segments,
creates a channel pair and moves one endpoint into the child. It creates one
initial thread with the fixed guarded stack and startup record, then starts it.
The parent closes its controller after start; the Job and scheduler retain the
running child independently. Temporary image mappings and construction handles
are released after setup.

The service announces READY over its delegated channel. Init then announces
readiness to the kernel control session. A PING challenge passes through init to
the service, and its PONG returns through init to the kernel. The normal release
path leaves both processes blocked on rooted message paths. It requires
`service_launch`, `multiservice_idle`, `init_ready`, `init_waiting` and
`boot_payload` PASS markers alongside normal memory verification.

A finite closure regression sends SHUTDOWN through init to the service. The
service closes its endpoint and exits zero; init closes its endpoints and exits
with the expected closure status. Dispatcher reclamation retires both tasks
before repeated frame, capability-node and process baseline checks. This finite
test is distinct from the resident normal boot.

The `service-wake-probe` build adds a second concrete software PING after the
first valid quiescent state. Its temporary software-source lease survives until
the matching PONG drains that work. Successful resumption additionally requires
`software_wake: PASS`; no hardware interrupt or external event producer is implied.

## Packaging preserves the firmware contract

`scripts/build-init-bundle.sh TARGET PROFILE [OUTPUT]` builds both executables
and emits `target/userspace-init/TARGET/PROFILE/boot.bundle` by default.
`make run INIT=1` passes that bundle as the RISC-V initrd;
`make x86-iso INIT=1` packages the same format under the UEFI filename
`BOOT.ELF`. The legacy filename does not identify the internal payload format.
Raw-ELF verification builds retain their existing delivery format. See
[boot-payload ownership](boot-payload.md) for permanent RO/NX source reservations.
