# Executable VMO publication

Syscall 16 irreversibly seals an owned-RAM VMO. Sealed backing can be mapped RX
through syscall 14; it cannot be written through VMO byte operations or new RW
user mappings. This is a whole-object state transition shared by every alias,
not a change to capability rights. Startup ABI v1 and minimal init are unchanged.

## Execution authority is requested before publication

Creation option `VMO_CREATE_EXECUTABLE = 1 << 1` requires
`VMO_CREATE_MAPPABLE = 1 << 0`. Valid creation options are 0, 1 and 3. Zero
preserves the original rights (783); 1 adds MAP (815); 3 adds MAP and EXECUTE
(831). Option 2 alone and unknown bits return -22. The new authority permits
publication of fresh memory, not access to arbitrary existing memory.

| Number | Call | Arguments | Success |
| --- | --- | --- | --- |
| 16 | `sys_vmo_seal` | Handle, options (`0`); other registers ignored | `0` |

Seal requires MANAGE and EXECUTE through a live capability. It validates the
handle, authority, owned-RAM backing and options before inspecting object
state. Repeating an authorized seal succeeds. Seal does not allocate, remove
mappings, attenuate handles, or grant rights. Physical/nonowning VMOs remain
excluded.

Map permission 5 means RX and requires READ, MAP and EXECUTE, plus sealed
backing. Existing permissions 1 (RO) and 3 (RW/NX) retain their rights checks.
RWX, execute-only and unknown permissions remain invalid. RX maps obey the
same logical-page, address, occupancy, table and mapping limits as other maps.
There is no unseal, partial seal or in-place permission conversion.

## Writable mappings block the entire transition

The VMO content lock protects sealed state and a global count of writable
mapping permits. Any permit blocks sealing with -16, including one held by
another process, a provisional map, or a mapping whose source handle was
closed, transferred or revoked. A writable subrange blocks the whole object.
RO mappings can remain installed across publication.

Acquire a nonallocating permit before fallible RW mapping preparation and
retain it in the successful mapping record. A failed installation releases it.
Exact unmap and last-thread reaping invalidate translations before releasing
permits; invalid or partial unmap cannot release them. Fallback destruction
destroys the address space before mapping owners. Killing a task alone does
not remove its mappings or release their permits.

The same lock protects ordinary VMO writes and sealing. Both owned-byte writes
and internal VMO write methods reject sealed objects, including empty writes.
The syscall checks write eligibility before user preflight or staging and the
write method checks again when committing. Existing WRITE-bearing aliases
retain their rights bits but cannot modify sealed backing. Reads, RO maps,
capability operations and exact unmap remain available subject to their usual
rights and liveness checks. Close or revocation never reverses sealing and
never removes an installed RX mapping.

Kernel outputs still require effective writable user pages. Capability inspect,
channel outputs and VMO read destinations cannot write through RO/RX aliases.
No writable user alias can coexist with sealing through these interfaces.
Permanent supervisor RAM aliases remain RW/NX and trusted kernel machinery;
this contract does not claim hardware immutability against arbitrary kernel
writes, physical wrappers or DMA.

## RX installation synchronizes local instruction fetch

Both production walkers synchronize instructions after executable leaves are
installed and local translation state is flushed. Failed installation performs
no instruction publication; non-executable maps require none. The hook is
infallible and allocates nothing after commit. Trusted ELF RX installation
uses the same path without adopting user sealing or user allocation charges.

RISC-V executes `fence.i` with compiler-visible memory effects. The initial
user-entry fence does not cover newly published code after a returning syscall.
x86 executes CPUID leaf zero with compiler fences, ordering code written
through the supervisor RAM alias before fetching it through the user alias.
Translation invalidation alone is not the documented instruction-publication
contract.

These hooks cover one active CPU with interrupts masked. Secondary harts stay
parked. Remote instruction synchronization, remote TLB invalidation, migration,
concurrent user writers and DMA coordination require separate work before SMP.

## State failures leave mappings and bytes unchanged

| Result | Meaning |
| --- | --- |
| -9 | Invalid handle, including high bits |
| -116 | Revoked capability or ancestor |
| -13 | Missing authority; sealed write/RW map; RX map of mutable backing |
| -16 | Seal blocked by a writable-mapping permit |
| -22 | Invalid seal/create options, wrong backing, unsupported map permissions or existing structural validation failures |

Existing range, collision, allocation and quota errors are unchanged. Map
retains structural validation and raw occupancy precedence, then checks state
before admitting mapping resources. A failed seal changes neither bytes nor
state. Failed maps unwind provisional permits and preserve existing mappings.
Zero-length reads retain the byte API's existing behavior; sealed writes fail
even when empty.

## Verification executes the published bytes

```sh
make run VMO_SEAL_PROBE=1
make x86-iso VMO_SEAL_PROBE=1
```

The separate Rust fixture copies relocation-free architecture-specific leaf
functions into fresh VMOs, seals them, maps RX and calls them. A second process
holds a writable alias to prove sealing is globally blocked until exact unmap.
The protocol checks denied writes, attenuated authority, shared executable
ownership and retained mappings after capability closure/revocation. It then
replaces function A with separately sealed function B at the same virtual
address in an already-running process and verifies B's different result.

Kernel evidence includes successful seal/map operations and verified execution
result messages. A warmup and three measured batches check frame, node,
process, scheduler and backing-budget restoration. The normal markers are
`memory_verification`, `vmo_sealing` and `boot_payload`, each with the
`[FERBER_TEST]` prefix and PASS result.

The separate `user-vmo-seal-write-probe` build attempts a store through a sealed
RX mapping. The expected fault must match user origin, instruction, address,
hardware cause and retained sealed RX metadata before emitting
`vmo_sealing_write_rx: PASS`. Deliberate fault halts do not establish reclamation.
Existing RO-write and NX-execute mapping probes remain separate regressions.
The published release ISO continues selecting minimal init.
