# Non-executable VMO mappings

Syscalls 14 and 15 install and remove owned-RAM VMO mappings in the calling
process. Permissions are RO or RW/NX. There is no executable publication,
remote mapping, automatic placement, overwrite, protection change or partial
unmap operation. Startup ABI v1 and minimal init's protocol are unchanged.

## MAP authority is requested at creation

`sys_vmo_create(size, 0)` preserves its original rights. Setting option
`VMO_CREATE_MAPPABLE = 1 << 0` adds MAP to the new capability. Other bits are
invalid. Existing handles cannot acquire MAP through MANAGE, duplication,
attenuation or transfer. No option grants EXECUTE. The byte API, exact logical
size, 16 MiB object maximum and backing charge remain unchanged.

| Number | Call | Arguments | Success |
| --- | --- | --- | --- |
| 14 | `sys_vmo_map` | Handle, VMO offset, virtual address, length, permissions, options | `0` |
| 15 | `sys_vmo_unmap` | Virtual address, length | `0` |

Map options must be zero. Permission 1 is RO and requires MAP plus READ;
permission 3 is RW/NX and requires MAP, READ and WRITE. Every other permission
encoding is rejected, including executable or write-only requests. All mapped
leaves are user accessible and non-executable. Physical/nonowning VMOs are
rejected even if their backing happens to have a RAM alias.

## Only complete logical pages are mapped

Offset, address and length must be page aligned. Length must be nonzero and at
most 16 MiB. Checked offset plus length must fit the exact logical VMO size.
A 4,097-byte object can map its first 4,096 bytes, but its final byte requires
byte I/O. A sub-page object cannot supply a mapping. Padding is not exposed.

The complete virtual interval must start at or above 4 KiB and end at or below
`1 << 38` on Sv39 or `1 << 47` on x86-64. No existing leaf, huge mapping,
supervisor translation or nonzero reserved PTE is overwritten. Registered user
stacks and their preceding guards are reserved independently of PTE contents;
an absent guard PTE is not free space. Loader and stack mappings are not user
mapping records and cannot be removed through syscall 15.

## Process records retain backing independently of handles

Each successful mapping records its base, length, VMO offset, permissions and
strong `Arc<Vmo>` owner. It does not retain the capability node or ancestry.
Close, transfer and revocation leave installed mappings valid. Revocation
blocks subsequent byte operations and new maps through the revoked node; it
does not remove mapped access. A read-only mapping can observe other writers.

Unmap requires an exact whole record belonging to the caller, with no handle
or MANAGE requirement. It remains available after close, transfer or revoke.
Partial ranges, combined adjacent records, holes and non-record mappings fail.
Translations are validated against every expected frame and permission before
removal; the record retains backing through invalidation and is released after
the process lock. The existing backing charge survives until the final
handle, message or mapping owner disappears.

Last-thread reaping drains dynamic mappings on the permanent-root dispatcher
stack before releasing their owners, even if a kernel reference retains the
dead Process. `Process::kill` only marks tasks dead; it does not tear down an
active root. Fallback field destruction destroys the address space before
backing owners.

## Page-table installation is prepared before publication

Both ports preflight the entire raw table range, count exact missing table
prefixes and allocate/zero them off-tree. Record capacity and table quota are
checked before publication. An error releases temporary tables without
changing existing PTEs or effective permissions. Commit allocates nothing.
x86 ancestor permission opening preserves sibling restrictions and occurs only
after fallible preparation. A full local translation flush follows publication.

Unmap first validates the complete record, then clears its leaves and prunes
empty owned lower-half tables. Reserved nonzero entries and other mappings
prevent pruning. Links are detached and local translation state invalidated
before retired table frames are freed. Root and shared upper-half tables are
never pruned; table operations never free VMO data frames.

## Aliases and tables have separate bounds

A process may hold at most 64 user mapping records and 16,384 mapped pages
(64 MiB), counting aliases. Its bounded user address space owns at most 256
private table frames, including its root and loader/stack tables. Shared kernel
tables are excluded. Pending table allocations count toward the limit and
return capacity on failure; pruning returns live capacity. Record-vector
capacity may remain cached up to its bounded limit.

These limits supplement the global user-VMO backing budget. They do not impose
global or hierarchical process/table quotas. Untrusted process creation must
settle that accounting before becoming available.

## Errors and precedence

| Result | Meaning |
| --- | --- |
| `-9` | Invalid handle or high bits |
| `-116` | Revoked capability or ancestor |
| `-13` | Missing MAP or requested access rights |
| `-22` | Unsupported options/permissions, wrong backing, alignment/logical bounds, protected range or invalid exact unmap |
| `-14` | Invalid user virtual interval |
| `-17` | Existing translation or mapping occupies the destination |
| `-90` | Per-operation mapping length exceeded |
| `-12` | Actual metadata or table-frame allocation failure |
| `-28` | Mapping-record, mapped-page or private-table limit exhausted |

Map checks handle validity and MAP, actual owned-RAM backing, options and
permission encoding, access rights, bounds, protected ranges and raw occupancy,
then limits and allocation. Guard reservations return -22; present mappings
return -17. Raw reservations are checked before quotas. Exact unmap validates
alignment and the user interval before record lookup. Failed preflight does
not partially remove mappings or release their backing.

## Execution remains single-CPU and interrupt-disabled

Syscalls retain their process root and trusted upper-half kernel entry/stack
mappings. Reaping switches to the permanent kernel root first. Inactive-root
safety depends on current activation flushing: no PCID contexts on x86 and
ASID zero with full fencing on RISC-V. Four-hart boots park secondary harts;
they do not establish remote shootdown safety.

Mapped user stores bypass the VMO content lock. Today's cooperative single-CPU
execution prevents concurrent user execution during kernel copies. This is
not a mapped-writer, DMA, interrupt-reentrancy or SMP guarantee. Executable
sealing, instruction-cache publication, partial unmap and coordinated mapped
revocation remain separate work.

## External probes exercise actual accesses

```sh
make run VMO_MAP_PROBE=1
make x86-iso VMO_MAP_PROBE=1
```

The separately compiled mapping fixture runs producer and consumer roles in
isolated processes. It uses volatile mapped accesses, transfers a read-only
mappable capability, checks shared updates and denied operations, closes handles
while retaining mappings, and unmaps/remaps different backing at the same VA.
Kernel evidence requires successful mapping operations, two clean exits and no
remaining records. Warmup plus three measured batches restore frame, node,
process, scheduler and backing-budget baselines. Successful boots require
`memory_verification`, `vmo_mapping` and `boot_payload` PASS markers, each with
the `[FERBER_TEST]` prefix.

Separate `user-vmo-map-write-probe` and `user-vmo-map-execute-probe` builds
validate a user RO-write or NX-execute protection fault at the fixture's known
PC and mapped address, with retained mapping metadata and hardware cause/error
bits checked. Their markers are `vmo_mapping_write_ro` and
`vmo_mapping_execute_nx`, alongside memory and payload markers. They halt at the
expected fault and do not claim process reclamation. Run each fault mode
separately. Both architecture smoke scripts exercise all three modes in debug
and release; `bash scripts/x86-iso-smoke.sh vmo-map` exercises ISO delivery.

The host paging runners compile both production walkers with controlled frame
allocation/aliases and privileged-operation instrumentation. They test failure
at each preparation depth, boundary crossings, raw reservations, huge leaves,
x86 ancestor restrictions, quotas, sparse churn and flush-before-free ordering.
The production syscall host harness additionally exercises the 16,384-page
alias boundary, failed admission without changes and capacity recovery after
unmap. These checks do not establish physical-hardware or SMP validation.
