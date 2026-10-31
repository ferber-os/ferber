# Job memory accounting

A Job is a container that authorizes processes and bounds their resource use. Its memory
usage includes descendant Jobs. Every heap backing page, owned VMO frame and private
page-table frame retains an allocation owner, even if a capability is transferred
elsewhere.

For example, giving a VMO to a process in another Job does not move its backing charge.
The original budget remains charged until the last allocation owner releases that
storage. This reference explains those charges and failure guarantees.

## Allocator storage belongs to its original budget

Each budget has its own buddy arena. A backing page is charged once, including
allocation headers, alignment padding, unused buddy blocks and fragmentation. A fully
coalesced page returns to the physical allocator and releases its charge. Large
allocations charge their entire rounded span. The payload ceiling remains 128 MiB, with
alignment at most 4 KiB; an aligned 4 KiB payload can require two pages because its
ownership header also needs storage.

Allocation headers retain the original budget. Freeing from another process refunds that
budget. Reallocation keeps it and accounts for the old and new storage simultaneously
when a replacement is needed; failure preserves the old allocation. Budget metadata is
allocated under the parent budget, avoiding a self-owning allocation cycle.

Process construction, task stacks, ELF population and syscall allocations run under the
target Job budget. Channel queue and waiter capacity growth uses the channel's original
budget; message storage retains its own allocation owner. Capability metadata and
temporary buffers use their allocation context. Global scheduler storage is explicitly
System-owned.

## Closing authority does not refund live storage

Owned VMOs and address spaces retain frame charges independently of handles. Transfer,
duplication, attenuation, close and revocation do not move or refund backing charges.
Mappings and queued capabilities can keep the backing alive after its creator exits.
Physical/MMIO VMOs do not charge device frames as owned RAM; their heap metadata remains
charged.

Killing a Job closes its budget before cascading termination. Closed ancestry rejects
new allocations, including reuse of cached arena blocks. Existing storage can still be
read, used and freed. Retained budget ancestry survives the structural Job until its
last allocation disappears. Frames are returned before their charges are released;
page-table removal also performs the required local TLB flush before freeing tables.

Fallible syscall allocation paths distinguish quota denial (`-28`) from physical or
allocator exhaustion (`-12`). The separate global user-created VMO backing limit still
applies; it is a policy counter rather than a second physical charge. Infallible Rust
allocation paths can still panic on exhaustion.

## Verification and concurrency boundaries

Host tests exercise the production ledger and allocator, ancestry denial, fragmentation,
reallocation, closed owners and concurrent admission. Paging tests check rollback and
free-before-refund ordering on both ports. Native boot checks use the actual global
allocator and reconcile System usage with bootstrap and allocated frame totals.

The `userspace-job-accounting-probe` build runs isolated producer and consumer Jobs. It
checks quota errors, capability transfer, creator teardown with retained foreign
backing, final release and repeated baseline restoration. Success requires
`[FERBER_TEST] job_accounting: PASS` and the normal memory and boot-payload markers.

The ledger is serialized, but production allocation context remains restricted to one
active CPU with interrupts masked. Scoped owner overrides and allocation failure capture
must end before switching tasks; scheduler assertions enforce that boundary. IRQ
reentrancy and SMP require per-CPU context before allocation can safely occur there.
Host concurrency tests do not establish production SMP safety.
