//! Ferber IPC
//! Channels (async, bytes + handles) + FastCall (sync, registers, ~50-150 cycles).
//! Handle transfer is move semantics. Sender loses handle on send.

pub mod channel;
pub mod fastcall;

#[allow(unused_imports)]
pub use channel::{
    Channel, ChannelEndpoint, ChannelError, ChannelMessage, SIGNAL_PEER_CLOSED, SIGNAL_READABLE,
    SIGNAL_WRITABLE,
};
#[allow(unused_imports)]
pub use fastcall::{fastcall, FastCallArgs, FastCallError, FastCallRet};

#[allow(dead_code)]
pub fn init() {}
