//! Bounded bidirectional channels. Each endpoint owns its incoming queue.
//! Transferring or discarding a message releases references; only revoke revokes.

#![allow(dead_code)]

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
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
    OutOfMemory,
}

#[derive(Debug)]
pub struct ChannelMessage {
    pub bytes: Vec<u8>,
    pub handles: Vec<Arc<CapNode>>,
}

impl ChannelMessage {
    pub fn new(bytes: Vec<u8>, handles: Vec<Arc<CapNode>>) -> Self {
        Self { bytes, handles }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    Open,
    PeerClosed,
}

static NEXT_CHANNEL_ID: AtomicU64 = AtomicU64::new(1);

pub const DEFAULT_MAX_QUEUE_DEPTH: usize = 64;
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 65536;
pub const DEFAULT_MAX_HANDLES: usize = 64;

/// One reusable ticket per task; stale notifications cannot signal a later wait.
pub struct WaitToken {
    state: AtomicU64,
}

impl WaitToken {
    pub const fn new() -> Self {
        Self {
            state: AtomicU64::new(0),
        }
    }

    pub fn start(&self, generation: u64) {
        assert!(
            generation != 0 && generation <= u64::MAX >> 1,
            "invalid wait generation"
        );
        self.state.store(generation << 1, Ordering::Release);
    }

    pub fn notify(&self, generation: u64) {
        if generation == 0 || generation > u64::MAX >> 1 {
            return;
        }
        let pending = generation << 1;
        let _ =
            self.state
                .compare_exchange(pending, pending | 1, Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn is_notified(&self, generation: u64) -> bool {
        generation != 0
            && generation <= u64::MAX >> 1
            && self.state.load(Ordering::Acquire) == (generation << 1 | 1)
    }
}

struct Waiter {
    token: Weak<WaitToken>,
    generation: u64,
}

fn notify(waiters: VecDeque<Waiter>) {
    for waiter in waiters {
        if let Some(token) = waiter.token.upgrade() {
            token.notify(waiter.generation);
        }
    }
}

struct PairState {
    queues: [VecDeque<ChannelMessage>; 2],
    waiters: [VecDeque<Waiter>; 2],
    open: [bool; 2],
}

/// Cancellation remains possible after the original capability node disappears.
#[derive(Clone)]
pub struct WaitRegistration {
    pair: Weak<SpinLock<PairState>>,
    side: usize,
}

impl WaitRegistration {
    pub fn unregister(&self, token: &Arc<WaitToken>, generation: u64) {
        if let Some(pair) = self.pair.upgrade() {
            let target = Arc::downgrade(token);
            pair.lock().waiters[self.side]
                .retain(|waiter| waiter.generation != generation || !waiter.token.ptr_eq(&target));
        }
    }
}

pub struct ChannelEndpoint {
    id: u64,
    side: usize,
    pair: Arc<SpinLock<PairState>>,
    max_queue_depth: usize,
    max_message_bytes: usize,
    max_handles: usize,
}

impl KernelObject for ChannelEndpoint {
    fn type_name(&self) -> &'static str {
        "Channel"
    }
}

impl ChannelEndpoint {
    pub fn create() -> Result<(Arc<Self>, Arc<Self>), ChannelError> {
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
    ) -> Result<(Arc<Self>, Arc<Self>), ChannelError> {
        let pair = Arc::try_new(SpinLock::new(PairState {
            queues: [VecDeque::new(), VecDeque::new()],
            waiters: [VecDeque::new(), VecDeque::new()],
            open: [true, true],
        }))
        .map_err(|_| ChannelError::OutOfMemory)?;
        let endpoint = |side| Self {
            id: NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed),
            side,
            pair: Arc::clone(&pair),
            max_queue_depth,
            max_message_bytes,
            max_handles,
        };
        let first = Arc::try_new(endpoint(0)).map_err(|_| ChannelError::OutOfMemory)?;
        let second = Arc::try_new(endpoint(1)).map_err(|_| ChannelError::OutOfMemory)?;
        Ok((first, second))
    }

    fn validate_size(&self, bytes: usize, handles: usize) -> Result<(), ChannelError> {
        if bytes > self.max_message_bytes {
            return Err(ChannelError::MessageTooLarge);
        }
        if handles > self.max_handles {
            return Err(ChannelError::TooManyHandles);
        }
        Ok(())
    }

    /// No source handles may be removed before admission and storage reservation.
    /// The callback must not acquire process/table locks or destroy kernel objects.
    pub fn send_with<E: From<ChannelError>>(
        &self,
        bytes: usize,
        handles: usize,
        prepare: impl FnOnce() -> Result<ChannelMessage, E>,
    ) -> Result<(), E> {
        self.validate_size(bytes, handles)?;
        let mut pair = self.pair.lock();
        let peer = 1 - self.side;
        if !pair.open[peer] {
            return Err(ChannelError::PeerClosed.into());
        }
        let queue = &mut pair.queues[peer];
        if queue.len() >= self.max_queue_depth {
            return Err(ChannelError::QueueFull.into());
        }
        queue
            .try_reserve(1)
            .map_err(|_| ChannelError::OutOfMemory)?;
        let message = prepare()?;
        assert_eq!(
            (message.bytes.len(), message.handles.len()),
            (bytes, handles)
        );
        queue.push_back(message);
        let waiters = core::mem::take(&mut pair.waiters[peer]);
        drop(pair);
        notify(waiters);
        Ok(())
    }

    /// Queue inspection and registration share a lock, closing the empty-check race.
    pub fn register_wait(
        &self,
        token: &Arc<WaitToken>,
        generation: u64,
    ) -> Result<bool, ChannelError> {
        let mut pair = self.pair.lock();
        if !pair.queues[self.side].is_empty() || !pair.open[1 - self.side] {
            return Ok(false);
        }
        let waiters = &mut pair.waiters[self.side];
        waiters.retain(|waiter| {
            waiter.token.upgrade().is_some_and(|ticket| {
                ticket.state.load(Ordering::Acquire) == waiter.generation << 1
            })
        });
        if !waiters.iter().any(|waiter| {
            waiter.generation == generation && waiter.token.ptr_eq(&Arc::downgrade(token))
        }) {
            waiters
                .try_reserve(1)
                .map_err(|_| ChannelError::OutOfMemory)?;
            waiters.push_back(Waiter {
                token: Arc::downgrade(token),
                generation,
            });
        }
        Ok(true)
    }

    pub fn unregister_wait(&self, token: &Arc<WaitToken>, generation: u64) {
        self.wait_registration().unregister(token, generation);
    }

    pub fn wait_registration(&self) -> WaitRegistration {
        WaitRegistration {
            pair: Arc::downgrade(&self.pair),
            side: self.side,
        }
    }

    /// On rejection the caller retains the message and drops it after unlocking.
    pub fn send(&self, message: ChannelMessage) -> Result<(), (ChannelError, ChannelMessage)> {
        let mut message = Some(message);
        let lengths = message.as_ref().unwrap();
        let result = self.send_with(lengths.bytes.len(), lengths.handles.len(), || {
            Ok(message.take().unwrap())
        });
        result.map_err(|error| (error, message.unwrap()))
    }

    /// Failed callbacks preserve the queue. Commit callbacks must preflight before
    /// mutating the message. Return residual ownership for destruction after locks.
    pub fn recv_with<R, E: From<ChannelError>>(
        &self,
        receive: impl FnOnce(&mut ChannelMessage) -> Result<R, E>,
    ) -> Result<(R, ChannelMessage), E> {
        let mut pair = self.pair.lock();
        let peer_open = pair.open[1 - self.side];
        let queue = &mut pair.queues[self.side];
        let message = queue.front_mut().ok_or(if peer_open {
            ChannelError::QueueEmpty
        } else {
            ChannelError::PeerClosed
        })?;
        let result = receive(message)?;
        Ok((result, queue.pop_front().unwrap()))
    }

    pub fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        self.recv_with(|_| Ok(())).map(|(_, message)| message)
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn state(&self) -> ChannelState {
        if self.is_peer_closed() {
            ChannelState::PeerClosed
        } else {
            ChannelState::Open
        }
    }

    pub fn signals(&self) -> u32 {
        let pair = self.pair.lock();
        let mut signals = 0;
        if !pair.queues[self.side].is_empty() {
            signals |= SIGNAL_READABLE;
        }
        if !pair.open[1 - self.side] {
            signals |= SIGNAL_PEER_CLOSED;
        } else if pair.queues[1 - self.side].len() < self.max_queue_depth {
            signals |= SIGNAL_WRITABLE;
        }
        signals
    }

    pub fn queue_len(&self) -> usize {
        self.pair.lock().queues[self.side].len()
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
        !self.pair.lock().open[1 - self.side]
    }
}

impl Drop for ChannelEndpoint {
    fn drop(&mut self) {
        let (discarded, waiters) = {
            let mut pair = self.pair.lock();
            pair.open[self.side] = false;
            (
                core::mem::take(&mut pair.queues[self.side]),
                core::mem::take(&mut pair.waiters),
            )
        };
        for queue in waiters {
            notify(queue);
        }
        drop(discarded);
    }
}

pub use ChannelEndpoint as Channel;

/// Exercise admission atomicity, retry, signal state, and destructor reentrancy.
#[cfg(not(test))]
pub fn self_test() {
    use crate::kernel::capabilities::{ObjectType, Rights, CAP_NODE_ALLOCATOR};
    use core::sync::atomic::AtomicUsize;

    let (first, second) = Channel::create_with_limits(1, 4, 1).unwrap();
    first
        .send(ChannelMessage::new(alloc::vec![1], Vec::new()))
        .unwrap();
    second
        .send(ChannelMessage::new(alloc::vec![2], Vec::new()))
        .unwrap();
    assert_eq!(first.signals() & SIGNAL_WRITABLE, 0);
    let full = first.send_with::<ChannelError>(0, 0, || panic!("full queue ran commit"));
    assert_eq!(full, Err(ChannelError::QueueFull));
    let failed = second.recv_with::<(), ChannelError>(|message| {
        assert_eq!(message.bytes, [1]);
        Err(ChannelError::MessageTooLarge)
    });
    assert_eq!(failed.unwrap_err(), ChannelError::MessageTooLarge);
    assert_eq!(second.queue_len(), 1);
    assert_eq!(second.recv().unwrap().bytes, [1]);
    assert_eq!(first.signals() & SIGNAL_WRITABLE, SIGNAL_WRITABLE);
    drop(second);
    assert_eq!(first.recv().unwrap().bytes, [2]);
    assert_eq!(first.recv().unwrap_err(), ChannelError::PeerClosed);
    assert_eq!(first.signals(), SIGNAL_PEER_CLOSED);

    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Reentrant {
        peer: alloc::sync::Weak<Channel>,
    }
    impl KernelObject for Reentrant {
        fn type_name(&self) -> &'static str {
            "Resource"
        }
    }
    impl Drop for Reentrant {
        fn drop(&mut self) {
            let peer = self.peer.upgrade().unwrap();
            let pair = peer
                .pair
                .try_lock()
                .expect("message dropped with pair locked");
            assert!(!pair.open[1 - peer.side]);
            DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let before = DROPS.load(Ordering::Relaxed);
    let nodes_before = CAP_NODE_ALLOCATOR.live_count();
    let (first, second) = Channel::create().unwrap();
    let object = Arc::new(Reentrant {
        peer: Arc::downgrade(&first),
    });
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(object, ObjectType::Resource, Rights::TRANSFER)
        .unwrap();
    first
        .send(ChannelMessage::new(Vec::new(), alloc::vec![node]))
        .unwrap();
    drop(second);
    assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);
    assert_eq!(CAP_NODE_ALLOCATOR.live_count(), nodes_before);
}
