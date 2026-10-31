# Uranium-238 v0.1.6

*Paired channels now carry bounded byte messages and capability references. Handle transfer removes the sender’s selected handle; this historical implementation also revokes capabilities in discarded messages.*

## What changed

- **Async Channels Architecture:** Implemented bidirectional paired endpoints (`ChannelEndpoint` in `wolfram/src/kernel/ipc/channel.rs`) with bounded message queues (64 items, 64 KiB total payload, 64 handles), backpressure enforcement on send, and signal bitmask transitions (`SIGNAL_READABLE`, `SIGNAL_WRITABLE`, `SIGNAL_PEER_CLOSED`).
- **Capability Move Semantics:** Integrated cross-process capability migration via `transfer_send` and `transfer_recv` in `wolfram/src/kernel/capabilities/handle_table.rs`. Capabilities require `Rights::TRANSFER` and are immediately stripped from the sender's `HandleTable` upon message dispatch, preventing duplicate possession.
- **Automatic Capability Revocation:** Implemented leak-free cleanup via `Drop for ChannelMessage`, ensuring unreceived in-flight capability nodes are automatically revoked (`node.revoke()`) if messages are dropped during queue teardown or channel destruction.
- **In-Kernel Boot Verification:** Added a comprehensive live 3-step test suite in `wolfram/src/main.rs` exercising paired endpoint message delivery, cross-process handle transfer with rights retention, and bidirectional peer-closed signaling upon endpoint drop.
- **Zero-Warning Toolchain Parity:** Maintained strict 1:1 behavioral parity and clean compilation with zero warnings under `-D warnings` across both RISC-V 64 (`riscv64gc-unknown-none-elf`) and x86-64 (`x86_64-unknown-none`).
