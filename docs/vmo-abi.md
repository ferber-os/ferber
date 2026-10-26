# VMO byte ABI

Syscalls 11–13 create and copy bytes through ordinary owned-RAM VMOs on both
architectures. They introduce no physical-address authority or resizing.
[Mapping syscalls 14–15](vmo-mapping.md) and [executable sealing](vmo-sealing.md)
provide separate mapping and publication contracts. Existing startup ABI v1 and init's
readiness protocol are unchanged.

## Calls complete exactly or return an error

| Number | Call | Arguments | Success |
| --- | --- | --- | --- |
| 11 | `sys_vmo_create` | Logical size, options | New handle |
| 12 | `sys_vmo_read` | Handle, offset, user destination, length | `0` |
| 13 | `sys_vmo_write` | Handle, offset, user source, length | `0` |

The existing six-register calling convention applies. Unused registers are
ignored. Creation options are zero for the original byte-only rights or
`VMO_CREATE_MAPPABLE = 1 << 0` to add MAP. Bit 1 requests EXECUTE and requires
bit 0; valid options are 0, 1 and 3. Other combinations are invalid. A handle must fit in `u32`; high bits are not
truncated. Read/write use explicit byte offsets, with no cursor, EOF truncation
or short success. Requests must fit wholly within the exact logical size.

Creation accepts 1 byte through 16 MiB. Backing rounds up to 4 KiB frames, with
every complete frame zeroed before exposure. Logical padding remains
inaccessible through byte calls. Allocation is scattered; no contiguity is
promised. Read/write length is at most 65,536 bytes, independent of the heap's
single-allocation ceiling.

Zero-length operations still validate the handle, revocation, required right,
backing kind and `offset <= size`. An otherwise valid write to sealed backing fails with -13, even when empty.
Eligible operations then return zero without examining the
user pointer or allocating scratch memory. An offset beyond the end is invalid
even for an empty copy.

## New authority covers new memory only

Creation follows channel creation: it creates a fresh object rather than naming
existing memory. The process receives exactly `READ | WRITE | DUPLICATE |
TRANSFER | INSPECT | MANAGE`. Zero-option creation receives no `MAP`, `EXECUTE`, `WAIT` or `SIGNAL`.
The explicit mappable option adds MAP only; options 3 add MAP and EXECUTE.
Executable mapping additionally requires irreversible object sealing.
Read requires READ; write requires WRITE. Existing attenuation, duplication,
revocation, inspection and channel move semantics apply without another VMO
handle representation. Generic inspect remains the eight-byte type/rights
record; VMO type is 2. No address or frame metadata crosses this ABI.

A read-only child can observe changes made through another writable alias.
Closing one handle does not revoke others. Revocation denies subsequent
capability operations, without clearing contents or destroying retained backing.
No promise is made that future concurrent revocation cancels an already
authorized copy.

The byte calls reject every physical/nonowning VMO, including physical RAM
wrappers. MMIO operations have side effects and require a separate device
access contract; having an ordinary RAM alias is not sufficient eligibility.
Internal physical VMO behavior is unchanged.

## Backing stays charged through final ownership

The global user-VMO limit is initialized once from managed pages:

```text
limit_pages = min(16,384, managed_pages / 4)
```

It is at most 64 MiB, with smaller machines limited to one quarter of managed
pages. The source is total managed pages, not changing free-page counts. A
zero-page budget is valid and rejects all nonzero user creations.

Charge complete backing pages before frame allocation. A nonallocating permit
belongs to the VMO, so construction failure releases it and final destruction
frees frames before releasing the charge. Duplicate and derived capabilities,
transferred handles, queued messages and surviving recipients retain the same
charge without recharging. Creator exit alone cannot release it.

Kernel-created VMOs and other kernel allocation paths are not charged here.
This bounds this allocation surface; it is neither hierarchical job quota
accounting nor a reservation guaranteeing free kernel RAM. Existing Job memory
limit/usage fields are not enforced by these calls. A request within the 16 MiB
object limit may still exceed the machine's budget.

## Staging preserves failure and aliasing contracts

A per-VMO content lock serializes kernel byte operations while size and frame
ownership remain immutable. Capability aliases retain the actual shared VMO.
No shared reference is cast to mutable and no unique Arc ownership is assumed.

Hold the process mapping-stability lock, retain the resolved node and release
the handle-table lock before copying. Preflight the complete logical VMO range,
all relevant supervisor RAM aliases and all user pages. A fallible heap scratch
buffer holds the whole bounded request; it is never a 64 KiB kernel-stack array.

Write snapshots the user source before committing to the VMO. Read snapshots
the VMO before writing the user destination. This permits overlapping physical
source/destination backing without violating nonoverlapping-copy contracts.
A read destination that repeats overlapping physical chunks is rejected: one
physical location cannot hold two incompatible output values. The shared
channel output validator applies the same rule.

Ordinary returned errors publish no handle and leave destinations unchanged.
Construction failures unwind pages, object/node allocation and budget permits.
Destruction happens after process/table/content guards are released. Copies
perform no allocation while holding the VMO content lock.

These guarantees require stable mappings and backing under the current
single-CPU, interrupt-disabled execution contract. They do not establish atomic
visibility to another CPU, recoverable hardware faults, mapped-writer
synchronization or DMA synchronization.

## Errors follow existing syscall conventions

| Result | Meaning |
| --- | --- |
| `-9` | Invalid handle, including high bits |
| `-116` | Revoked capability or ancestor |
| `-13` | Required right absent or write to sealed backing |
| `-22` | Invalid options, zero creation size, wrong type/backing, physical VMO, invalid logical range or contradictory output aliases |
| `-90` | Per-object or per-copy size limit exceeded |
| `-14` | Invalid user mapping, address or permission |
| `-12` | Actual frame, metadata, object or staging allocation failure |
| `-24` | Full handle table |
| `-28` | User-VMO budget or capability-node capacity exhausted |

Read/write check handle validity, rights and backing before copy length and
logical range, then check write eligibility before zero length, user preflight and allocation. Creation
validates options and size, checks capacity before expensive allocation, and
rechecks before publishing its privately constructed node.

## Mappings have independent ownership

[Non-executable mapping](vmo-mapping.md) uses process records retaining shared
VMO backing independently of capability nodes. Existing mappings survive
close, transfer and revoke until exact unmap or process teardown. Page-table
preparation, reclamation and quotas are separate from backing charges.

[Executable sealing](vmo-sealing.md) blocks all VMO writes and new writable
mappings after verifying no writable mapping survives in any process. RX
installation synchronizes local instruction fetch. The content lock does not
serialize user stores through installed mappings; future SMP needs remote
synchronization and a concurrent mapped-writer contract.

## External verification keeps init separate

```sh
make run VMO_PROBE=1
make x86-iso VMO_PROBE=1
```

The `userspace-vmo-probe` configuration loads the separate
`userspace/vmo-probe/` executable into two isolated processes. Their bootstrap
channel messages select producer and consumer roles. The producer creates and
checks a zeroed VMO, writes across a page boundary, duplicates and attenuates
a capability, then transfers the read-only child. The consumer verifies the
bytes and denied write. A subsequent producer write is visible through the
consumer's shared backing even after the producer closes its owner. Both exit
with no remaining handles. Kernel evidence requires actual successful VMO and
capability/channel syscalls, not just two zero exit statuses.

A warmup and three measured repetitions restore frame, node, process, scheduler
and user-VMO budget baselines. The scenario requires exactly
`memory_verification: PASS`, `vmo_syscalls: PASS` and `boot_payload: PASS`, each
with the `[FERBER_TEST]` prefix. A physical x86-64 boot on the 32 GiB MSI
workstation reached all three markers with the pre-commit `0.2.10` banner,
including the repeated ownership and budget checks. Production init and its
startup protocol are unchanged; the release artifact continues to select
minimal init.
