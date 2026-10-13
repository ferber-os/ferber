//! Async bidirectional channel. Carries bytes + handles.
//! Bounded queue. Backpressure on send when full.
//! Handle transfer is a move. Sender loses the handle.

#![allow(dead_code)]

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::capabilities::{CapNode, KernelObject};
use crate::kernel::sync::SpinLock;

pub const SIGNAL_READABLE: u32 = 1 << 0;
pub const SIGNAL_WRITABLE: u32 = 1 << 1;
pub const SIGNAL_PEER_CLOSED: u32 = 1 << 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelError {
    PeerClosed,
    QueueFull,
    QueueEmpty,
    MessageTooLarge,
    TooManyHandles,
    InsufficientRights,
}

pub struct ChannelMessage {
    pub bytes: Vec<u8>,
    pub handles: Vec<NonNull<CapNode>>,
}

unsafe impl Send for ChannelMessage {}
unsafe impl Sync for ChannelMessage {}

impl ChannelMessage {
    pub fn new(bytes: Vec<u8>, handles: Vec<NonNull<CapNode>>) -> Self {
        Self { bytes, handles }
    }
}

impl Drop for ChannelMessage {
    fn drop(&mut self) {
        for node in &self.handles {
            unsafe {
                node.as_ref().revoke();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    Open,
    PeerClosed,
}

static NEXT_CHANNEL_ID: AtomicU64 = AtomicU64::new(1);

pub const DEFAULT_MAX_QUEUE_DEPTH: usize = 64;
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 65536; // 64 KiB
pub const DEFAULT_MAX_HANDLES: usize = 64;

pub struct ChannelEndpoint {
    id: u64,
    peer: Option<Weak<SpinLock<ChannelEndpoint>>>,
    queue: VecDeque<ChannelMessage>,
    max_queue_depth: usize,
    max_message_bytes: usize,
    max_handles: usize,
    state: ChannelState,
    signals: u32,
}

unsafe impl Send for ChannelEndpoint {}
unsafe impl Sync for ChannelEndpoint {}

impl KernelObject for ChannelEndpoint {
    fn type_name(&self) -> &'static str {
        "Channel"
    }
}

impl ChannelEndpoint {
    pub fn create() -> (Arc<SpinLock<ChannelEndpoint>>, Arc<SpinLock<ChannelEndpoint>>) {
        Self::create_with_limits(
            DEFAULT_MAX_QUEUE_DEPTH,
            DEFAULT_MAX_MESSAGE_BYTES,
            DEFAULT_MAX_HANDLES,
        )
    }

    pub fn create_with_limits(
        max_queue_depth: usize,
        max_message_bytes: usize,
        max_handles: usize,
    ) -> (Arc<SpinLock<ChannelEndpoint>>, Arc<SpinLock<ChannelEndpoint>>) {
        let id1 = NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed);
        let id2 = NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed);

        let end1 = Arc::new(SpinLock::new(ChannelEndpoint {
            id: id1,
            peer: None,
            queue: VecDeque::new(),
            max_queue_depth,
            max_message_bytes,
            max_handles,
            state: ChannelState::Open,
            signals: SIGNAL_WRITABLE,
        }));

        let end2 = Arc::new(SpinLock::new(ChannelEndpoint {
            id: id2,
            peer: Some(Arc::downgrade(&end1)),
            queue: VecDeque::new(),
            max_queue_depth,
            max_message_bytes,
            max_handles,
            state: ChannelState::Open,
            signals: SIGNAL_WRITABLE,
        }));

        end1.lock().peer = Some(Arc::downgrade(&end2));

        (end1, end2)
    }

    pub fn send(&mut self, msg: ChannelMessage) -> Result<(), ChannelError> {
        if self.state == ChannelState::PeerClosed {
            return Err(ChannelError::PeerClosed);
        }

        if msg.bytes.len() > self.max_message_bytes {
            return Err(ChannelError::MessageTooLarge);
        }

        if msg.handles.len() > self.max_handles {
            return Err(ChannelError::TooManyHandles);
        }

        let peer_arc = match self.peer.as_ref().and_then(|p| p.upgrade()) {
            Some(peer) => peer,
            None => {
                self.state = ChannelState::PeerClosed;
                self.signals |= SIGNAL_PEER_CLOSED;
                self.peer = None;
                return Err(ChannelError::PeerClosed);
            }
        };

        let mut peer = peer_arc.lock();
        if peer.state == ChannelState::PeerClosed {
            self.state = ChannelState::PeerClosed;
            self.signals |= SIGNAL_PEER_CLOSED;
            return Err(ChannelError::PeerClosed);
        }

        if peer.queue.len() >= peer.max_queue_depth {
            return Err(ChannelError::QueueFull);
        }

        peer.queue.push_back(msg);
        peer.signals |= SIGNAL_READABLE;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<ChannelMessage, ChannelError> {
        match self.queue.pop_front() {
            Some(msg) => {
                if self.queue.is_empty() {
                    self.signals &= !SIGNAL_READABLE;
                }
                Ok(msg)
            }
            None => {
                self.signals &= !SIGNAL_READABLE;
                if self.state == ChannelState::PeerClosed {
                    Err(ChannelError::PeerClosed)
                } else {
                    Err(ChannelError::QueueEmpty)
                }
            }
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn state(&self) -> ChannelState {
        self.state
    }

    pub fn signals(&self) -> u32 {
        self.signals
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn max_queue_depth(&self) -> usize {
        self.max_queue_depth
    }

    pub fn max_message_bytes(&self) -> usize {
        self.max_message_bytes
    }

    pub fn max_handles(&self) -> usize {
        self.max_handles
    }

    pub fn is_peer_closed(&self) -> bool {
        self.state == ChannelState::PeerClosed
    }
}

impl Drop for ChannelEndpoint {
    fn drop(&mut self) {
        if let Some(peer_weak) = self.peer.take() {
            if let Some(peer_arc) = peer_weak.upgrade() {
                let mut peer = peer_arc.lock();
                peer.state = ChannelState::PeerClosed;
                peer.signals |= SIGNAL_PEER_CLOSED;
                peer.peer = None;
            }
        }
    }
}

pub use ChannelEndpoint as Channel;
