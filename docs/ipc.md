# Wolfram IPC

Wolfram provides two inter-process communication primitives. Both are strictly capability-controlled.

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
- **Errors:** `PeerClosed`, `QueueFull`, `QueueEmpty`, `MessageTooLarge`, `TooManyHandles`, `InsufficientRights`.
- **Capability Move Semantics:** `transfer_send()` requires `TRANSFER` and moves an owned node reference out of the sender's table. Independently duplicated handles remain present. `transfer_recv()` moves ownership into the receiving table; on failure it returns the error and owned reference.
- **Leak-Free Drop Revocation:** If a message carrying capabilities is dropped before delivery (queue full, teardown, closed peer), `Drop for ChannelMessage` invokes `node.revoke()`, invalidating the nodes, their aliases, and their descendants before releasing the message's owned references. This trusted-kernel revocation does not require a user `MANAGE` right.

Messages keep their nodes, ancestry, and object backing alive while queued.
Reclamation follows the final owned reference, not numeric handle closure alone.
Rejected sends return `(ChannelError, ChannelMessage)` with the carried nodes
already revoked. Callers must release the endpoint lock before dropping the
returned message, since final object destruction can acquire kernel locks.
The undelivered-message revocation policy needs review before public channel
syscalls; channel syscalls remain future work.

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
