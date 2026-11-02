# Device authority and mappings

A Device handle names one kernel-approved hardware assignment. It does not grant
PCI configuration access or accept arbitrary physical addresses. Ordinary VMO
creation cannot manufacture a Device. The concrete backing is checked in addition
to its capability type tag (`6`).

These interfaces are groundwork for the console services. Normal boot does not yet
grant hardware assignments to userspace. IRQ binding, IRQ acknowledgement, hardware
time queries and deadlines currently return `-95`; armed interrupt delivery and an
interactive console are not available through this ABI yet.

## A single driver owns mutable assignment state

Querying a device does not claim it. The first mapping, DMA or start operation binds the
assignment to one process. Ownership cannot move to another process afterward. A validated mutation can retain
that claim if later page-table allocation fails; the same owner can retry.
Process construction can explicitly transfer an unclaimed Device handle; channel
messages cannot carry device authority.

Device mappings and DMA leases have lifetimes independent of handle slots. Closing
a handle releases that reference; it does not remove existing mappings or DMA
leases. A process-owned record retains cleanup responsibility. Revoking any device
node used by that owner contains the whole assignment, including views created
through other aliases. The assignment cannot then be restarted through a sibling
handle. This is a single-driver hardware boundary, rather than independent aliases
to independently controllable devices.

Before returning from capability revocation, the kernel reconciles device owners,
stops affected assignments and removes their register mappings. Process death and
final teardown perform the same containment before releasing process memory.
Failed hardware quiescence quarantines the backend and DMA backing instead of
returning uncertain pages to the allocator.

## Syscalls expose approved regions and owned DMA backing

All unused arguments must be zero. Integers in descriptors are little-endian.

| Call | Arguments | Required device rights |
| --- | --- | --- |
| `24` query | Device, region index, output, `48`, `0`, `0` | INSPECT |
| `25` map | Device, request, `32`, `0`, `0`, `0` | MAP plus READ, and WRITE for RW |
| `26` unmap | Device, base, exact length, `0`, `0`, `0` | MAP |
| `27` DMA map | Device, VMO, request, `32`, output, `24` | MANAGE |
| `28` DMA unmap | Device, lease ID, `0`, `0`, `0`, `0` | MANAGE |
| `31` control | Device, operation, `0`, `0`, `0`, `0` | MANAGE |

Control operation `1` claims and starts an assignment; `2` stops an already claimed assignment. DMA
controllers require DMA backing before start; explicitly non-DMA devices do not.
Mapping changes are allowed only while isolated or stopped.

The 32-byte device mapping request contains a region index (`u32`), four zero
bytes, user virtual address (`u64`), exact region length (`u64`) and permissions
(`u64`: `1` RO or `3` RW). The entire approved region must be mapped. Addresses and
lengths are page-aligned, within userspace and free of existing mappings and stack
guards. Mappings are NX and retain physical VMOs under the assignment, without treating
the aperture as allocator-owned RAM. There are at most 16 device mappings and 128 MiB of device
apertures per process, accounted separately from owned RAM VMO mappings. User-copy
helpers reject device pointers because they require an ordinary supervisor RAM alias.

The 48-byte query result contains version `1` (`u32`), region kind (`u32`), length
(`u64`), allowed permissions (`u64`), width, height, pixel stride and pixel format
(four `u32`s), followed by eight zero bytes. Kinds are registers (`1`), framebuffer
(`2`) and UART (`3`). Non-framebuffer geometry is zero. Framebuffer pixel formats
are RGB (`1`) and BGR (`2`), both four bytes per pixel. Physical addresses are absent.

## DMA leases retain backing and writable permits

The 32-byte DMA request contains VMO offset, length, a zero reserved field and
permissions (four `u64`s). Device READ is `1`; device WRITE is `2`. Offset and length
are page-aligned and must identify allocator-owned VMO backing. The VMO capability
requires MAP and the corresponding READ/WRITE rights.

The 24-byte result contains lease ID, kernel-chosen IOVA and length (three `u64`s).
The kernel preflights this output before installing DMA. Each assignment permits at
most 64 leases and 4 MiB of backing. Unmapping requires the exact lease ID and a
stopped or isolated assignment. Invalidation must complete before the lease releases
its backing. A device-write permit prevents executable sealing until that lease is
safely removed, even if the original VMO handle has been closed.

Neither host regressions nor dormant backend code establish physical DMA isolation.
Platform discovery, firmware handoff, requester quiescence and interrupt routing must
be verified before an assignment is exposed to an actual driver.
