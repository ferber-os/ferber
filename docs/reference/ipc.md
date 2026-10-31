# Inter-process communication

A channel connects two endpoints. Programs send bounded messages and, for eligible
objects, move capability handles through them. Ferrum uses channels for filesystem
requests, service control and bootstrap communication. Each operation requires authority
through the calling process’s endpoint handle.

For example, a filesystem client sends READ on its delegated session and blocks for the
reply. Another process with no session capability cannot make that request by inventing
the client's handle number. See [filesystem sessions](initramfs.md).

## Channel operations and bounds

| Operation | Syscall | Required endpoint rights | Behavior |
| --- | --- | --- | --- |
| Create pair | 7 | New object | Returns two endpoints |
| Send | 8 | WRITE | Enqueues or returns an error |
| Receive | 9 | READ | Receives immediately or returns an error |
| Blocking receive | 10 | READ, WAIT | Suspends until data, closure or cancellation |
| Wait-many | 23 | READ, WAIT on every endpoint | Returns a ready index without consuming data |

Each incoming queue holds at most 64 messages. A message contains at most 65,536 bytes
and 64 transferred capabilities. The kernel represents bytes as `Vec<u8>` and owned node
references as `Vec<Arc<CapNode>>`.

READABLE (`1 << 0`) reflects queued messages. WRITABLE (`1 << 1`) reflects queue space.
PEER_CLOSED (`1 << 2`) records closure of the opposite endpoint. A receiver drains
queued messages before observing peer closure. Internal errors include `PeerClosed`,
`QueueFull`, `QueueEmpty`, `MessageTooLarge`, `TooManyHandles`, `InsufficientRights` and
`OutOfMemory`; [the ABI](syscall-abi.md) defines errno values.

## Transfer moves one reference and preserves failures

A payload capability requires TRANSFER. Successful send removes the selected source
slots only after message admission. Independently duplicated aliases remain valid. A
failed send preserves every source slot. Receive validates output ranges and free handle
slots before publishing bytes or handles; failure preserves the head message, table and
user output.

Messages retain nodes, ancestry and backing until delivery or discard. Discard releases
those references without revoking other aliases or descendants. Explicit revocation
requires MANAGE. A revoked queued node can still be received and closed, but grants no
usable authority.

Queued payloads are restricted to checked VMOs and concrete leaf Resource objects.
Channel endpoints and task objects are rejected to avoid ownership cycles. [Direct
delegation into a draft process](task-creation.md) has a separate contract and can move
an endpoint before the child starts.

A shared pair-state lock protects both queues and endpoint liveness. Failed kernel
`send()` returns `(ChannelError, ChannelMessage)`, preserving the message's owned
references. Discarded messages are detached while locked and destroyed after unlocking;
opposite endpoints do not acquire one another's locks.

## Blocking waits retain the original authority

A blocked receiver keeps its original capability identity and wait generation. Closing
its watched slot or revoking its node/ancestry cancels the wait. Reusing the number for
a different capability cannot change the suspended operation's authority. Cancellation
does not consume a message or modify receive outputs.

Wait-many watches 1–16 distinct endpoint identities with one generation. A queued
message or closed peer is ready. Every handle is validated before reporting a ready
index; closing or revoking any member cancels the entire wait. All member registrations
are removed on completion, failure or teardown.

The cooperative scheduler checks the graph of blocked receivers and potential producers
to distinguish legitimate idle from deadlock. Filesystem sessions create no implicit
wake source. See [init](init.md) for rooted waiting.

## FastCall is a separate kernel primitive

FastCall transfers five argument words and two result words through registers. Its
kernel model blocks the caller and directly hands execution to the target thread without
routing through the ready queue. This synchronous thread-donation model differs from
queued asynchronous messages.

The primitive is exercised by kernel tests. A complete userspace FastCall ABI,
interrupt-driven service delivery and measured end-to-end latency are not established by
those tests. No end-to-end latency benchmark or cache-line traffic guarantee is
established.

See [verification](verification.md) for channel failure, concurrency and boot coverage.
Production scheduling remains cooperative on one active CPU.
