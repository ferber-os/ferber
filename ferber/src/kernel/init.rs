//! Initial Ferrum process: explicit bootstrap ownership and terminal waiting.

use super::capabilities::{CapNode, ObjectType, Rights, CAP_NODE_ALLOCATOR};
use super::elf_load::{self, Admitted};
use super::ipc::channel::{Channel, ChannelMessage};
use super::memory::user_mem::copy_from_user;
use super::sync::SpinLock;
use super::task::Job;
use alloc::sync::{Arc, Weak};

#[path = "../../../shared/init_protocol.rs"]
mod protocol;

#[path = "../../../shared/boot_bundle.rs"]
mod bundle;
#[path = "../../../shared/service_protocol.rs"]
mod service;

const MAX_SERVICES: usize = 9;
struct ServiceRecord {
    process_id: u64,
    endpoint: Weak<Channel>,
    ping: bool,
    pong: bool,
    exit: Option<usize>,
}

pub(crate) struct Session {
    pub admitted: Admitted,
    pub node: Weak<CapNode>,
    pub peer: Option<Arc<Channel>>,
    pub closure_probe: bool,
    pub parked_once: bool,
    pub exit_status: Option<usize>,
    root: Option<Weak<SpinLock<Job>>>,
    launch: Option<Arc<SpinLock<Job>>>,
    source: Option<super::scheduler::SourceLease>,
    software: Option<super::scheduler::SourceLease>,
    ready_seen: bool,
    pong_seen: bool,
    services: [Option<ServiceRecord>; MAX_SERVICES],
    declared_services: usize,
    unexpected_exit: Option<usize>,
    challenge_pending: bool,
    challenge: u32,
    woke: bool,
}

static SESSION: SpinLock<Option<Session>> = SpinLock::new(None);

fn admit(root: &Arc<SpinLock<Job>>, closure_probe: bool) -> Session {
    let bytes = super::boot_payload::bytes().expect("boot payload required");
    let bundle = bundle::Bundle::parse(bytes).expect("invalid init boot bundle");
    let launch = root
        .lock()
        .create_child_job("services", 16 * 1024 * 1024)
        .expect("service launch Job");
    let job = super::syscall_task::job_capability(&launch).expect("service launch authority");
    let image_size = bundle.archive.len().div_ceil(4096) * 4096;
    let mut image = super::memory::Vmo::create(image_size).expect("service image VMO");
    image.write(0, bundle.archive).expect("copy service image");
    image.seal().expect("seal service image");
    let image = Arc::new(image);
    let image = CAP_NODE_ALLOCATOR
        .alloc_root(
            image,
            ObjectType::Vmo,
            Rights::READ | Rights::MAP | Rights::INSPECT | Rights::TRANSFER,
        )
        .expect("service image authority");
    let (user, peer) = Channel::create().expect("init bootstrap channel");
    peer.send(ChannelMessage::new(
        protocol::encode_length(protocol::START, bundle.archive.len() as u64)
            .unwrap()
            .to_vec(),
        alloc::vec![job, image],
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
    let admitted =
        elf_load::admit_named(root, bundle.init, Some(node), "init").expect("load init ELF");
    let source =
        super::scheduler::register_source(&peer, super::scheduler::SourceKind::TerminalBootstrap)
            .expect("init root event source");
    Session {
        admitted,
        node: weak,
        peer: Some(peer),
        closure_probe,
        parked_once: false,
        exit_status: None,
        root: Some(Arc::downgrade(root)),
        launch: Some(launch),
        source: Some(source),
        software: None,
        ready_seen: false,
        pong_seen: false,
        services: core::array::from_fn(|_| None),
        declared_services: 0,
        unexpected_exit: None,
        challenge_pending: false,
        challenge: 74,
        woke: false,
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
    let current_job = super::scheduler::current_process()
        .lock()
        .job()
        .and_then(|job| job.upgrade());
    let mut session = SESSION.lock();
    let Some(session) = session.as_mut() else {
        return false;
    };
    if session.admitted.process_id == process_id && session.admitted.thread_id == thread_id {
        session.exit_status = Some(status);
        return true;
    }
    if let Some(record) = session
        .services
        .iter_mut()
        .flatten()
        .find(|r| r.process_id == process_id)
    {
        record.exit = Some(status);
        return true;
    }

    if current_job
        .as_ref()
        .zip(session.launch.as_ref())
        .is_some_and(|(job, launch)| Arc::ptr_eq(job, launch))
    {
        session.unexpected_exit = Some(if status == 0 { 256 } else { status });
        return true;
    }
    false
}

/// Record only successful channel sends by the admitted init or a process in its service Job.
pub(crate) fn observe(nr: usize, args: [usize; 6], result: isize) {
    if nr != super::syscall::SYS_CHAN_SEND
        || result != 0
        || args[2] != service::SIZE
        || args[4] != 0
    {
        return;
    }
    let (init_id, launch) = {
        let session = SESSION.lock();
        let Some(session) = session.as_ref() else {
            return;
        };
        let Some(launch) = session.launch.as_ref() else {
            return;
        };
        (session.admitted.process_id, Arc::clone(launch))
    };
    let process = super::scheduler::current_process();
    let (process_id, in_service_job, endpoint, bytes) = {
        let process = process.lock();
        if process.state() != super::task::ProcessState::Running {
            return;
        }
        let Some(job) = process.job().and_then(|job| job.upgrade()) else {
            return;
        };
        let in_service_job = Arc::ptr_eq(&job, &launch);
        if process.id() != init_id && !in_service_job {
            return;
        }
        let handle = match u32::try_from(args[0]) {
            Ok(handle) => handle,
            Err(_) => return,
        };
        let table = process.handle_table().lock();
        let Ok(node) = table.lookup(handle) else {
            return;
        };
        if !node.rights().contains(super::capabilities::Rights::WRITE) {
            return;
        }
        let Some(endpoint) = node.object_arc::<Channel>() else {
            return;
        };
        let mut bytes = [0; service::SIZE];
        if unsafe { copy_from_user(process.address_space(), &mut bytes, args[1]) }.is_err() {
            return;
        }
        (process.id(), in_service_job, endpoint, bytes)
    };
    let Some((op, value)) = service::decode(&bytes) else {
        return;
    };
    let mut guard = SESSION.lock();
    let Some(session) = guard.as_mut() else {
        return;
    };
    let Some(session_launch) = session.launch.as_ref() else {
        return;
    };
    if !Arc::ptr_eq(session_launch, &launch) {
        return;
    }
    if process_id == init_id {
        if op == service::PING && value == session.challenge && session.challenge_pending {
            for record in session.services.iter_mut().flatten() {
                if record
                    .endpoint
                    .upgrade()
                    .is_some_and(|e| e.wait_registration().peer_of(&endpoint.wait_registration()))
                {
                    record.ping = true;
                }
            }
        }
        return;
    }
    if !in_service_job {
        return;
    }
    if op == service::READY && value == 0 {
        if session.services.iter().flatten().any(|r| {
            r.process_id == process_id
                || r.endpoint
                    .upgrade()
                    .is_some_and(|e| Arc::ptr_eq(&e, &endpoint))
        }) {
            return;
        }
        if let Some(slot) = session.services.iter_mut().find(|r| r.is_none()) {
            *slot = Some(ServiceRecord {
                process_id,
                endpoint: Arc::downgrade(&endpoint),
                ping: false,
                pong: false,
                exit: None,
            });
        }
    } else if op == service::PONG && session.challenge_pending && value == session.challenge {
        if let Some(record) = session
            .services
            .iter_mut()
            .flatten()
            .find(|r| r.process_id == process_id)
        {
            if record.ping
                && record
                    .endpoint
                    .upgrade()
                    .is_some_and(|e| Arc::ptr_eq(&e, &endpoint))
            {
                record.pong = true;
            }
        }
    }
}

pub(crate) fn take() -> Option<Session> {
    SESSION.lock().take()
}

impl Session {
    pub(crate) fn test_session(
        admitted: Admitted,
        node: Weak<CapNode>,
        peer: Arc<Channel>,
    ) -> Self {
        Self {
            admitted,
            node,
            peer: Some(peer),
            closure_probe: false,
            parked_once: false,
            exit_status: None,
            root: None,
            launch: None,
            source: None,
            software: None,
            ready_seen: false,
            pong_seen: false,
            services: core::array::from_fn(|_| None),
            declared_services: 0,
            unexpected_exit: None,
            challenge_pending: false,
            challenge: 74,
            woke: false,
        }
    }
    fn all_services(&self, predicate: impl Fn(&ServiceRecord) -> bool) -> bool {
        self.declared_services > 0
            && self.services.iter().flatten().count() == self.declared_services
            && self.services.iter().flatten().all(predicate)
    }
    pub fn ready(&self) -> bool {
        self.ready_seen
            && self.pong_seen
            && !self.challenge_pending
            && self.all_services(|r| r.ping && r.pong && r.exit.is_none())
    }
    pub(crate) fn service_waits_match(&self) -> bool {
        if self
            .launch
            .as_ref()
            .is_some_and(|job| job.lock().process_count() != self.declared_services)
        {
            return false;
        }
        self.all_services(|r| {
            r.endpoint
                .upgrade()
                .is_some_and(|e| super::scheduler::blocked_on(r.process_id, &e))
        })
    }
    pub(crate) fn service_exit_status(&self) -> Option<usize> {
        self.unexpected_exit
            .or_else(|| {
                self.services
                    .iter()
                    .flatten()
                    .filter_map(|r| r.exit)
                    .find(|s| *s != 0)
            })
            .or_else(|| self.all_services(|r| r.exit == Some(0)).then_some(0))
    }
    pub fn announced_ready(&self) -> bool {
        self.ready_seen
    }
    pub fn begin_shutdown(&mut self) -> bool {
        let Some(peer) = &self.peer else { return false };
        peer.send(ChannelMessage::new(
            service::encode(service::SHUTDOWN, 0).to_vec(),
            alloc::vec![],
        ))
        .is_ok()
    }
    pub fn before_quiescent(&mut self) -> bool {
        if !cfg!(feature = "service-wake-probe") || self.woke {
            return false;
        }
        let Some(peer) = &self.peer else { return false };
        self.software = Some(
            super::scheduler::register_source(peer, super::scheduler::SourceKind::Software)
                .expect("pending service software wake"),
        );
        self.challenge = 75;
        self.woke = true;
        self.pong_seen = false;
        for record in self.services.iter_mut().flatten() {
            record.ping = false;
            record.pong = false;
        }
        peer.send(ChannelMessage::new(
            service::encode(service::PING, self.challenge).to_vec(),
            alloc::vec![],
        ))
        .expect("software service ping");
        self.challenge_pending = true;
        true
    }
    #[cfg(feature = "service-wake-probe")]
    pub fn software_woke(&self) -> bool {
        self.woke && self.pong_seen && self.software.is_none()
    }
}

pub(crate) fn pump() -> bool {
    let mut session = SESSION.lock();
    let Some(session) = session.as_mut() else {
        return false;
    };
    if session.exit_status.is_some() || session.service_exit_status().is_some() {
        return false;
    }
    let Some(peer) = &session.peer else {
        return false;
    };
    let Ok(message) = peer.recv() else {
        return false;
    };
    if !message.handles.is_empty() {
        panic!("init bootstrap failed");
    }
    if !session.ready_seen {
        let (op, count) = protocol::decode_record(&message.bytes)
            .unwrap_or_else(|_| panic!("init bootstrap failed"));
        if op != protocol::READY
            || count == 0
            || count > MAX_SERVICES as u64
            || session.services.iter().flatten().count() != count as usize
            || session
                .launch
                .as_ref()
                .is_some_and(|job| job.lock().process_count() != count as usize)
        {
            panic!("init bootstrap failed");
        }
        session.declared_services = count as usize;
        session.ready_seen = true;
        peer.send(ChannelMessage::new(
            service::encode(service::PING, session.challenge).to_vec(),
            alloc::vec![],
        ))
        .expect("initial service ping");
        session.challenge_pending = true;
    } else {
        if session.pong_seen
            || !session.challenge_pending
            || !session.all_services(|r| r.ping && r.pong && r.exit.is_none())
            || service::decode(&message.bytes) != Some((service::PONG, session.challenge))
        {
            panic!("init bootstrap failed");
        }
        session.challenge_pending = false;
        session.pong_seen = true;
        drop(session.software.take());
    }
    true
}

impl Drop for Session {
    fn drop(&mut self) {
        drop(self.source.take());
        drop(self.software.take());
        if let Some(launch) = self.launch.take() {
            launch.lock().kill();
            if let Some(root) = self.root.as_ref().and_then(Weak::upgrade) {
                let id = launch.lock().id();
                let detached = root.lock().detach_child_job(id);
                drop(detached);
            }
            drop(launch);
        }
    }
}
