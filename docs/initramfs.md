# Ferrum resolves boot files through a read-only service

The boot envelope supplies an init ELF and an opaque, independently versioned
[initramfs archive](initramfs-format.md). The kernel validates the envelope and
loads init; it does not resolve archive paths or expose pathname syscalls.
Firmware transport and its 4 MiB total limit remain unchanged.

Init receives its services Job and a sealed owned archive VMO with READ, MAP,
INSPECT and TRANSFER. It maps the archive temporarily to load `sbin/initfs`,
then relinquishes its mapping and moves the VMO to the filesystem service.
Initfs maps it RO/NX for its lifetime and receives no task-creation authority.

Init predelegates server session endpoints into the filesystem draft. One client
session belongs to init; eight more are available for explicit delegation to
configured services. Endpoints are installed through process-give before start,
never transferred in queued channel messages. The filesystem startup record
`FRFSBOOT` carries exact archive length and receiver-local server slots, with
one attached archive VMO. Configured services receive a `FRSVBOOT` record with
their optional receiver-local filesystem slot before announcing READY.

## Session channels carry streaming requests

Filesystem wire v1 uses a 32-byte little-endian header: magic `FRFS0001` (8 bytes),
version (u16), opcode (u16), request ID (u32), status (i32), flags (u32) and
session-local object ID (u64). Responses echo the request and operation. Requests
have zero status and no attached capabilities. Messages are at most 4128 bytes.

Each session permits eight open objects. IDs increase monotonically and are never
reused; an ID is meaningful only through its session channel. Possession of that
channel grants read-only access to the archive namespace. Root is `/`; all other
paths use canonical archive-relative names. IDs are not transferable capabilities.

- OPEN (1): object zero, flags file=1 or directory=2, body is the path. Reply
  supplies a nonzero ID and a 16-byte body: kind u32, reserved zero u32, length u64.
- READ (2): object ID, flags zero, body offset u64, requested length u32 and
  reserved zero u32. Length is 1–4096. Reply contains actual bytes and EOF flag=1
  when the file end is reached. Reading beyond EOF is invalid.
- LIST (3): directory ID, body cursor u32 and reserved zero u32. Reply returns
  one immediate child: next cursor u32, kind u32, length u64 and basename bytes.
  An empty EOF response ends enumeration. Entries follow archive order.
- CLOSE (4): object ID, no body or flags. Reply has no body.
- STAT (5): object ID, no body or flags. Reply uses OPEN's kind/length body.

Protocol errors use negative status: invalid arguments -22, missing path -2,
kind mismatch -22, unknown ID -9, open-table exhaustion -24 and ID exhaustion -28.
Clients allow one outstanding request and validate response IDs, lengths and flags.
Malformed envelopes, attached capabilities or failed replies disconnect only that
session. Peer closure releases all its open state. Initfs rotates client selection
while giving control messages priority; it blocks through channel wait-many rather
than polling. Loss of a required startup session causes init to fail the boot.

## Configuration delegates filesystem access explicitly

`etc/services.conf` is at most 4096 bytes, with lines at most 320 bytes:

```text
version 1
service alpha libexec/services/alpha fs
service beta libexec/services/beta none
```

Blank lines and whole-line comments beginning `#` are accepted. Names are unique,
1–31 ASCII letters/digits/underscore/hyphen. Paths are canonical archive paths.
There must be 1–8 services. Unknown directives, versions, extra fields and access
modes fail startup. `fs` explicitly delegates a session; `none` delegates no file
access. Arguments, environment, restart and dependency policies are not defined.

Init reads configuration and streams executable files through initfs. It prepares
all configured process drafts before starting any configured child. The existing
ELF parser, VMO zero-fill, sealing and task authority checks still apply. Each
child receives only its explicit bootstrap and optional filesystem endpoint.

The filesystem plus configured services participate in origin-checked readiness,
challenge fanout and rooted idle. Init sends each child the original challenge,
validates all replies, then responds to the kernel. Any required failure tears
down the services Job; the shutdown regression checks every tracked child's
successful exit and resource restoration.

This service exposes immutable boot files. Persistent storage, writable filesystem
operations, new runtime session connections and general VFS/mount policy remain
future work. Archive validation does not authenticate boot contents. The implementation passed physical x86-64 validation with pre-bump `0.3.0`
metadata. The newly versioned `0.3.1` ISO has not been physically tested; RISC-V
remains QEMU-only.
