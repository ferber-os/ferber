# Wolfram IPC

Wolfram provides two inter-process communication primitives. Both are strictly capability-controlled.

## Channels

Channels are asynchronous, bidirectional message pipes connecting paired endpoints (`ChannelEndpoint`).

- **Payload:** Raw bytes (`Vec<u8>`) + capability handles (`Vec<NonNull<CapNode>>`).
- **Queue Limits:**
  - `DEFAULT_MAX_QUEUE_DEPTH`: 64 messages per endpoint queue.
  - `DEFAULT_MAX_MESSAGE_BYTES`: 65,536 bytes (64 KiB) per message.
  - `DEFAULT_MAX_HANDLES`: 64 capability handles per message.
- **Signals:**
  - `SIGNAL_READABLE` (`1 << 0`): Set when messages are available in the queue; cleared when empty.
  - `SIGNAL_WRITABLE` (`1 << 1`): Set when the queue can accept new messages.
  - `SIGNAL_PEER_CLOSED` (`1 << 2`): Raised when the remote peer endpoint is dropped or closed.
- **Errors:** `PeerClosed`, `QueueFull`, `QueueEmpty`, `MessageTooLarge`, `TooManyHandles`, `InsufficientRights`.
- **Capability Move Semantics:** Handle transfer is an atomic move. Sending a handle removes it immediately from the sender's `HandleTable` via `transfer_send()`.
- **Leak-Free Drop Revocation:** If a message carrying capabilities is dropped before delivery (queue full, teardown, closed peer), `Drop for ChannelMessage` invokes `node.revoke()`, immediately invalidating all in-transit capability nodes.

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
