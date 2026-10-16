# Resident Syscall ABI

Wolfram uses Linux-style argument registers with its own syscall numbers.
This ABI currently runs one resident task on one active CPU. It does not provide
Linux or musl compatibility, scheduling, or complete task/object reclamation.

## Calling Convention

| Architecture | Number | Arguments 0–5 | Signed result |
| --- | --- | --- | --- |
| RISC-V 64 | `a7` | `a0`–`a5` | `a0` |
| x86-64 | `rax` | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` | `rax` |

Enter through `ecall` on RISC-V and `syscall` on x86-64. Unused arguments are
ignored. Returning calls preserve general registers other than the result and
the x86 `syscall` architectural clobbers (`rcx`, `r11`). x86 return flags are
sanitized. FP/vector state is outside the resident ABI.

## Operations

| Number | Name | Arguments | Successful result |
| --- | --- | --- | --- |
| `0` | Yield hint | None | `0`, immediately |
| `1` | Thread exit | Exit status | Does not return; marks the thread Dead and halts |
| `2` | Handle close | Handle | `0` |
| `3` | Handle duplicate | Handle | New handle number |
| `4` | Capability inspect | Handle, output address | `0` |

Every handle is resolved through the trusted calling process's table. Processes
are born with zero handles; delegation is explicit. Handle zero is valid.
Handle arguments greater than `u32::MAX` are rejected rather than truncated.

Close releases one table entry without requiring additional rights. A revoked
handle can be closed. Close neither revokes the node nor frees its object.
Closing an already empty slot is an error.

Duplicate requires `DUPLICATE` and inserts an alias to the same node with the
same rights and type. It does not consume a capability-node allocation. Closing
one alias leaves the others usable; revoking the shared node invalidates all
aliases. The table currently has 256 slots. Vacant numbers can be reused, so a
previously closed number can later refer to another capability.

Inspect requires `INSPECT`. It copies this fixed eight-byte record to the
supplied writable user address, which need not be aligned:

| Byte offset | Field | Encoding |
| --- | --- | --- |
| `0` | Object type | Little-endian `u32` |
| `4` | Rights bitmask | Little-endian `u32` |

Object types are Process `0`, Thread `1`, VMO `2`, Channel `3`, Job `4`, and
Resource `5`. Rights use the bit positions in the
[capability model](capability-model.md#rights). No pointers, internal node IDs,
parent data, or names are exposed, and the object pointer is not dereferenced.

The copy checks the entire range before writing. Null, overflowing, supervisor,
unmapped, or nonwritable destinations fail without changing any output byte.
Mappings/backing remain stable under the current resident-task constraints;
concurrent unmapping and recoverable copy faults remain future work.

## Errors and Validation Order

| Result | Meaning |
| --- | --- |
| `-9` | Invalid handle, including nonzero bits above bit 31 |
| `-13` | Required right is absent |
| `-14` | Inspection output range is invalid or not writable user memory |
| `-24` | Handle table is full |
| `-38` | Unknown syscall number |
| `-116` | Capability node is revoked |

For duplication and inspection, validate the handle, then node validity, then
required rights. Only then check destination capacity or output memory.
Rejected calls leave the table unchanged. Inspection-copy failures also leave
the output unchanged. These error values do not imply Linux ABI compatibility.

## Resident Verification

Normal probes receive explicitly delegated fixture handles after their empty
birth-table check. They exercise permitted and denied operations, invalid and
revoked handles, table exhaustion/recovery, user-copy boundaries, and complete
handle cleanup. Probe bookkeeping is separate from runtime dispatch.
The resident test observer diagnoses a nonzero exit status or incomplete
coverage as a failed run, with a kernel panic rather than success markers.

A successful normal boot emits exactly one of each marker:

```text
[WOLFRAM_TEST] privilege_switch_roundtrip: PASS
[WOLFRAM_TEST] handle_syscalls: PASS
```

Isolation, RX-write, and NX-execute probes remain separate builds without these
handle fixtures. Their existing protection-fault markers remain unchanged.
