//! Initial Ferrum process: explicit bootstrap ownership and terminal waiting.

use super::capabilities::{CapNode, ObjectType, Rights, CAP_NODE_ALLOCATOR};
use super::elf_load::{self, Admitted};
use super::ipc::channel::{Channel, ChannelMessage};
use super::sync::SpinLock;
use super::task::Job;
use alloc::sync::{Arc, Weak};

#[path = "../../../shared/init_protocol.rs"]
mod protocol;

pub(crate) struct Session {
    pub admitted: Admitted,
    pub node: Weak<CapNode>,
    pub peer: Option<Arc<Channel>>,
    pub closure_probe: bool,
    pub parked_once: bool,
    pub exit_status: Option<usize>,
}

static SESSION: SpinLock<Option<Session>> = SpinLock::new(None);

fn admit(root: &Arc<SpinLock<Job>>, closure_probe: bool) -> Session {
    let (user, peer) = Channel::create().expect("init bootstrap channel");
    peer.send(ChannelMessage::new(
        protocol::encode(protocol::START).unwrap().to_vec(),
        alloc::vec::Vec::new(),
    ))
    .unwrap_or_else(|(error, _)| panic!("init START: {error:?}"));
    let node = CAP_NODE_ALLOCATOR
        .alloc_root(
            user,
            ObjectType::Channel,
            Rights::READ | Rights::WRITE | Rights::WAIT,
        )
        .expect("init bootstrap capability");
    let weak = Arc::downgrade(&node);
    let admitted = elf_load::admit_named(
        root,
        super::boot_payload::bytes().expect("boot payload required"),
        Some(node),
        "init",
    )
    .expect("load init ELF");
    Session {
        admitted,
        node: weak,
        peer: Some(peer),
        exit_status: None,
        closure_probe,
        parked_once: false,
    }
}

pub fn run(root: &Arc<SpinLock<Job>>) -> ! {
    // Warm admission/dispatcher capacities, then check an actual suspended init can resume and retire.
    super::scheduler::run_init_closure_probe(admit(root, true));
    let frames = super::memory::bitmap::stats().free_pages;
    let nodes = CAP_NODE_ALLOCATOR.live_count();
    let processes = root.lock().process_count();
    super::scheduler::run_init_closure_probe(admit(root, true));
    assert_eq!(
        super::memory::bitmap::stats().free_pages,
        frames,
        "init closure frame leak"
    );
    assert_eq!(
        CAP_NODE_ALLOCATOR.live_count(),
        nodes,
        "init closure capability leak"
    );
    assert_eq!(
        root.lock().process_count(),
        processes,
        "init closure process leak"
    );
    super::scheduler::run_init(admit(root, false))
}

pub(crate) fn install(session: Session) {
    let mut current = SESSION.lock();
    assert!(current.is_none());
    *current = Some(session);
}

pub(crate) fn record_exit(status: usize) -> bool {
    let (process_id, thread_id) = super::scheduler::current_identity();
    let mut session = SESSION.lock();
    if let Some(session) = session.as_mut().filter(|session| {
        session.admitted.process_id == process_id && session.admitted.thread_id == thread_id
    }) {
        session.exit_status = Some(status);
        true
    } else {
        false
    }
}

pub(crate) fn take() -> Option<Session> {
    SESSION.lock().take()
}

impl Session {
    pub fn ready(&self) -> bool {
        let Some(peer) = &self.peer else { return false };
        let Ok(message) = peer.recv() else {
            return false;
        };
        message.handles.is_empty()
            && protocol::decode(&message.bytes) == Ok(protocol::READY)
            && peer.queue_len() == 0
            && !peer.is_peer_closed()
    }
}
