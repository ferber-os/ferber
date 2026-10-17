# Ferber IPC

Ferber provides two inter-process communication primitives. Both are strictly capability-controlled.

## Channels

Channels are asynchronous, bidirectional message pipes connecting paired endpoints (`ChannelEndpoint`).

- **Payload:** Raw bytes (`Vec<u8>`) + capability handles (`Vec<Arc<CapNode>>`).
- **Queue Limits:**
  - `DEFAULT_MAX_QUEUE_DEPTH`: 64 messages per endpoint queue.
  - `DEFAULT_MAX_MESSAGE_BYTES`: 65,536 bytes (64 KiB) per message.
  - `DEFAULT_MAX_HANDLES`: 64 capability handles per message.
- **Signals:**
  - `SIGNAL_READABLE` (`1 << 0`): Set when messages are available in the queue; cleared when empty.
  - `SIGNAL_WRITABLE` (`1 << 1`): Set when the queue can accept new messages.
  - `SIGNAL_PEER_CLOSED` (`1 << 2`): Raised when the remote peer endpoint is dropped or closed.
- **Errors:** `PeerClosed`, `QueueFull`, `QueueEmpty`, `MessageTooLarge`, `TooManyHandles`, `InsufficientRights`, `OutOfMemory`.
- **Capability Move Semantics:** `transfer_send()` requires `TRANSFER` and moves an owned node reference out of the sender's table. Independently duplicated handles remain present. `transfer_recv()` moves ownership into the receiving table; on failure it returns the error and owned reference.
- **Message discard:** Releases owned capability references without revoking nodes, surviving aliases, or descendants. Explicit user revocation requires `MANAGE`.

Messages keep their nodes, ancestry, and object backing alive while queued.
Reclamation follows the final owned reference, not numeric handle closure alone.
Kernel `send()` rejects with `(ChannelError, ChannelMessage)` and leaves the
carried nodes valid. A shared pair-state lock protects queues and endpoint
liveness without acquiring an opposite endpoint lock. Queued messages discarded
on endpoint destruction are detached under that lock and destroyed after unlocking.

Resident syscalls `7`–`9` create pairs and send/receive without blocking on both
architectures. Send requires endpoint `WRITE` and payload `TRANSFER`; receive
requires endpoint `READ`. Failed sends preserve source slots, and failed receives
preserve the head message, table, and output buffers. Queued messages drain before
receive reports peer closure. Payloads are restricted to checked VMO and current
leaf Resource backing; graph-owning objects need an ownership-cycle policy before
transfer. See the [resident ABI](syscall-abi.md) for buffers, errors, and limits.
Blocking receive and scheduler integration remain future work.

## FastCall

FastCall is a synchronous, register-only IPC mechanism for latency-critical paths.

- **Performance:** Designed for ~50-150 cycles. Bypasses the scheduler ready queue entirely.
- **Model:** Synchronous thread donation. The caller thread transitions to `ThreadState::Blocked` and donates its remaining time slice directly to the target thread (`ThreadState::Running`).
- **Register Protocol:**
  - Arguments: 5 words in registers (`arg0..arg4` via `FastCallArgs`).
  - Return: 2 words in registers (`ret0..ret1` via `FastCallRet`).
  - Zero memory allocation, zero cache line churn.
- **Use Case:** Kernel interrupt dispatch, hardware event notifications, and capability hot paths only. Not for general application code.

*See [docs/architecture.md](architecture.md) for architectural context.*
