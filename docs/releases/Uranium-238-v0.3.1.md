# Uranium-238 v0.3.1

*Ferrum now reads named boot files through a userspace filesystem service and starts multiple configured services. An independently versioned archive and bounded channel wait sets support that startup without kernel pathname lookup.*

## Boot delivery and filesystem layout have separate contracts

Boot envelope v2 carries an initial ELF and opaque initramfs bytes. The existing
BootInfo v3 transport, source reservations, permanent supervisor RO/NX aliases
and 4 MiB total payload ceiling remain. UEFI uses `BOOT.ELF`; RISC-V uses initrd.
The kernel validates the envelope and loads init, without parsing archive paths.

Ferber initramfs format v1 has a little-endian header, sorted fixed-size path
index, dense string table and uncompressed file data. The allocation-free parser
validates the entire archive before lookup. It bounds entry counts, path sizes
and depth and rejects malformed paths, missing parents, duplicate or unsorted
entries, unsupported kinds, overflow, overlaps, nonzero padding and trailing data.
Only files and directories are supported. There is no public archive acronym or
filename extension, compression, ownership database or authentication claim.

Host tooling packs reproducibly, lists and verifies through the production parser,
and extracts into a new destination using no-follow directory operations and
exclusive creation. Failed packing preserves existing output. Extraction does
not overwrite existing files or restore archive-supplied ownership metadata.

## Init delegates bytes and sessions to its filesystem process

Bootstrap protocol v3 keeps startup ABI v1 unchanged. Init receives the services
Job and a sealed archive VMO with READ, MAP, INSPECT and TRANSFER rights. It maps
the archive to construct `sbin/initfs`, then relinquishes its mapping and moves
the archive to the filesystem process. Initfs retains an RO/NX mapping for its
lifetime and receives no process-creation authority.

Session pairs are created and delegated before process start through the existing
task ABI. General queued channel-endpoint transfer remains prohibited. OPEN,
READ, LIST, CLOSE and STAT messages operate through a delegated session channel.
Open IDs increase monotonically and have meaning only within that session;
they are not independently transferable capabilities. Reads are bounded to
4096 bytes, directories are paginated and each session has eight open slots.

Malformed messages, unexpected capabilities and failed replies disconnect the
offending client. They do not terminate the entire filesystem service. The
server rotates client selection and blocks on wait sets rather than polling.

## Wait sets extend the existing cancellation and idle rules

Syscall 23, `sys_chan_wait_many`, accepts 1–16 channel handles and returns the
lowest ready input index without consuming a message. Readability includes
queued data and peer closure. Every handle requires READ and WAIT; all entries
are validated before reporting readiness and endpoint aliases are rejected.

One task token and generation cover every registration. Setup failure rolls
back earlier registrations; return, cancellation and teardown remove the whole
set. Close, slot reuse or revocation of any watched capability cancels the wait.
The scheduler includes all members in producer/source analysis. Filesystem
waiting gains no implicit external wake root. Existing syscall contracts remain.

## Configured services are prepared before any of them execute

Init reads `etc/services.conf` and executable bytes through initfs. The bounded
configuration specifies up to eight required services and explicit filesystem
access (`fs` or `none`). The default image contains initfs and two configured
service executables, with filesystem access granted to one. Init prepares all
configured drafts before starting any configured child. Existing ELF validation,
zero-filled BSS, segment permissions and executable sealing remain enforced.

Readiness, challenge fanout and idle evidence are bound to each real child
process and its exact endpoint. Aggregate readiness must match actual Job
membership. Missing or invalid files, failed construction, unexpected exits and
unrooted waits fail startup. The shutdown probe reclaims init and every service
and checks restored allocation, capability and process accounting.

During validation, two defects were fixed: reusing a PONG buffer while forwarding
the next child's PING, and admitting an earlier configured service before a later
executable had been validated. Adversarial checks also exposed filesystem-wide
termination on malformed clients or full reply queues; these failures are now
contained to their sessions.

## Verification separates implementation evidence from release metadata

Local host tests passed for archive/tool round trips and malformed data,
filesystem protocol/session isolation and stale IDs, configuration, scheduler
wait sets, channel races and allocation failures, task construction, hierarchical
accounting and ELF parsing/loading. Both kernel targets passed Clippy with
warnings denied. QEMU coverage on both architectures includes init/service
execution, software wake, protection faults, replay rejection, missing or invalid
configured executables, malformed-client containment, reply-queue flooding and
disconnected waits. Release ISO boots passed for init and software wake. The
freestanding ELF, shell syntax, workflow lint and strict log-parser checks passed.
Hosted CI results are not part of this local validation record.

A physical x86-64 boot on the 32 GiB MSI B650M-A PRO WIFI reached:

```text
[FERBER_TEST] memory_verification: PASS
[FERBER_TEST] boot_payload: PASS
[FERBER_TEST] service_launch: PASS
[FERBER_TEST] multiservice_idle: PASS
[FERBER_TEST] init_ready: PASS
[FERBER_TEST] init_waiting: PASS
```

The run managed 32218 MiB (31.46 GiB), 8247922 frames and 24 extents. It used the
pre-bump `0.3.0` banner and validates the initramfs implementation build. The
newly versioned `0.3.1` ISO is not claimed as physically tested. The photo does
not establish an exact source-commit or ISO-hash association. The dedicated
software-wake build was not flashed, and no physical
RISC-V validation is claimed.

Integrated allocation-failure injection during multi-service preparation/start
and quota exhaustion with several retained drafts remain coverage gaps; lower
level task and allocator failure tests passed. Execution remains cooperative,
single-CPU and integer-only with interrupts masked. Writable or persistent
filesystems, general VFS/mount policy, dynamic filesystem-session connections,
restart/dependency policy, recoverable user faults, preemption and SMP remain
outside this release. AArch64 is the next prioritized port and is not implemented.

Both crates advance to `0.3.1` without dependency changes. See the
[archive format](../reference/initramfs-format.md), [filesystem protocol](../reference/initramfs.md),
[syscall ABI](../reference/syscall-abi.md) and [init contract](../reference/init.md).
