# The boot envelope delivers init and a named archive

Init builds consume two independent payloads in one bounded firmware file:
Ferrum init's static ELF and an [immutable initramfs archive](initramfs-format.md).
The envelope identifies those bootstrap roles; the archive contains named files
and directories. Neither format authenticates executable bytes. BootInfo v3
still transports one opaque file, bounded to 4 MiB including all envelope
metadata, padding, init and archive bytes.

## Envelope v2 preserves the bounded handoff

`shared/boot_bundle.rs` defines the allocation-free parser. Integers are
little-endian. The header is 32 bytes followed by exactly two 32-byte entries.

| Header offset | Width | Value |
| --- | --- | --- |
| 0 | 8 | Magic `FRBOOT01` |
| 8 | 4 | Version `2` |
| 12 | 4 | Entry count `2` |
| 16 | 8 | Exact total file length |
| 24 | 8 | Reserved, zero |

| Entry offset | Width | Value |
| --- | --- | --- |
| 0 | 4 | Kind: first `1` (init ELF), then `3` (archive) |
| 4 | 4 | Flags, zero |
| 8 | 8 | File offset |
| 16 | 8 | Nonzero exact file length |
| 24 | 8 | Reserved, zero |

Offsets are 8-byte aligned and cannot overlap metadata or preceding payloads.
Inter-entry padding is zero; the archive ends exactly at the declared length.
Unknown/duplicate kinds, v1 envelopes, nonzero reserved fields, overflow,
truncated ranges, oversized input and trailing bytes are rejected. The firmware
handoff needs no change to carry this version.

## Authority belongs to each explicitly delegated reference

The kernel loads only init's envelope ELF. A dedicated services Job provides
bounded launch authority. Init's separate bootstrap v3 START record carries
exact archive byte length and two attached handles in order: weak Job authority,
then a fresh owned, page-rounded, sealed archive VMO. Its rights are READ, MAP,
INSPECT and TRANSFER; it has neither WRITE nor EXECUTE authority.

Init maps the archive read-only, validates it and locates `sbin/initfs`. It loads
that ELF using existing task/VMO syscalls, explicitly installs filesystem session
channels and starts the child. Init unmaps the archive and moves its source VMO
through initfs's control channel. Initfs validates its received archive and keeps
an RO mapping after closing the source handle. Mapping ownership retains the
backing without an open handle.

Init obtains `etc/services.conf` and configured executable bytes through its
filesystem session. It constructs each required service with fresh segment
VMOs, sealed executable backing, a guarded stack and explicit capabilities.
Configured services receive their own filesystem channel only when requested
by configuration. No filesystem channel or archive mapping is inherited.
See [the filesystem service](initramfs.md) and [init](init.md) for those protocols.

## Packaging remains independent of the kernel

`scripts/build-init-bundle.sh TARGET PROFILE [OUTPUT]` builds init, initfs and
resident service executables, stages the initial layout, emits an extensionless
`initramfs` archive beside the output, and writes `boot.bundle` by default under
`target/userspace-init/TARGET/PROFILE/`.

The initial tree has `sbin/initfs`, `etc/services.conf`, two executable names
under `libexec/services/`, and test data under `share/`. The same test service
binary occupies both configured executable names; they become separate isolated
processes. Init stays in the envelope rather than being duplicated in the archive.

`make run INIT=1` passes the envelope as the RISC-V initrd;
`make x86-iso INIT=1` packages it under the legacy UEFI transport name `BOOT.ELF`.
Raw-ELF verification builds retain their existing delivery format. Source bytes
remain reserved through the permanent supervisor RO/NX alias, as described in
[boot-payload ownership](boot-payload.md). Replacing archive contents does not
require recompiling the kernel, subject to the total envelope and Job budgets.
