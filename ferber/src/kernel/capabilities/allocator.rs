use super::{CapError, CapNode, KernelObject, ObjectType, Rights};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub const NODE_CAPACITY: usize = 1024;

/// Bounded live-node factory. The heap owns storage; permits own the quota.
pub struct CapNodeAllocator {
    live: AtomicUsize,
    next_id: AtomicU64,
}

pub(super) struct NodePermit {
    allocator: &'static CapNodeAllocator,
}

impl Drop for NodePermit {
    fn drop(&mut self) {
        let previous = self.allocator.live.fetch_sub(1, Ordering::Release);
        debug_assert!(previous != 0, "capability quota underflow");
    }
}

impl CapNodeAllocator {
    const fn new() -> Self {
        Self {
            live: AtomicUsize::new(0),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn live_count(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    fn reserve(&'static self) -> Result<(NodePermit, u64), CapError> {
        self.live
            .try_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                (live < NODE_CAPACITY).then_some(live + 1)
            })
            .map_err(|_| CapError::NodeCapacity)?;
        let permit = NodePermit { allocator: self };
        let id = self
            .next_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| CapError::NodeCapacity)?;
        Ok((permit, id))
    }

    fn allocate(
        &'static self,
        object: Arc<dyn KernelObject>,
        object_type: ObjectType,
        rights: Rights,
        parent: Option<Arc<CapNode>>,
    ) -> Result<Arc<CapNode>, CapError> {
        let (permit, id) = self.reserve()?;
        Arc::try_new(CapNode::new(
            id,
            rights,
            object,
            object_type,
            parent,
            permit,
        ))
        .map_err(|_| CapError::OutOfMemory)
    }

    pub fn alloc_root(
        &'static self,
        object: Arc<dyn KernelObject>,
        object_type: ObjectType,
        rights: Rights,
    ) -> Result<Arc<CapNode>, CapError> {
        if Rights::from_bits(rights.bits()).is_none() {
            return Err(CapError::InvalidRights);
        }
        self.allocate(object, object_type, rights, None)
    }

    pub(super) fn alloc_child(
        &'static self,
        parent: &Arc<CapNode>,
        rights: Rights,
    ) -> Result<Arc<CapNode>, CapError> {
        self.allocate(
            Arc::clone(&parent.object),
            parent.object_type,
            rights,
            Some(Arc::clone(parent)),
        )
    }
}

pub static CAP_NODE_ALLOCATOR: CapNodeAllocator = CapNodeAllocator::new();

/// Exercise reservation rollback without consuming the production factory's IDs or quota.
pub(super) fn self_test() {
    use alloc::alloc::{AllocError, Allocator, Layout};
    use alloc::vec::Vec;
    use core::ptr::NonNull;

    struct FailingAllocator;
    // This allocator never creates storage, so there is no successful allocation to deallocate.
    unsafe impl Allocator for FailingAllocator {
        fn allocate(&self, _layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            Err(AllocError)
        }
        unsafe fn deallocate(&self, _pointer: NonNull<u8>, _layout: Layout) {
            unreachable!("failing allocator cannot own storage");
        }
    }

    static TEST: CapNodeAllocator = CapNodeAllocator::new();
    static EXHAUSTED_IDS: CapNodeAllocator = CapNodeAllocator {
        live: AtomicUsize::new(0),
        next_id: AtomicU64::new(u64::MAX),
    };
    let mut permits = Vec::new();
    let mut previous_id = 0;
    for _ in 0..NODE_CAPACITY {
        let (permit, id) = TEST.reserve().unwrap();
        assert!(id > previous_id);
        previous_id = id;
        permits.push(permit);
    }
    assert_eq!(TEST.reserve().err(), Some(CapError::NodeCapacity));
    assert_eq!(TEST.live_count(), NODE_CAPACITY);
    drop(permits.pop());
    let (permit, _) = TEST.reserve().unwrap();
    drop(permit);
    drop(permits);
    assert_eq!(TEST.live_count(), 0);
    assert_eq!(EXHAUSTED_IDS.reserve().err(), Some(CapError::NodeCapacity));
    assert_eq!(
        EXHAUSTED_IDS.live_count(),
        0,
        "ID exhaustion leaked a reservation"
    );
    assert_eq!(EXHAUSTED_IDS.next_id.load(Ordering::Relaxed), u64::MAX);

    let object: Arc<dyn KernelObject> = Arc::new(super::Resource);
    let (permit, id) = TEST.reserve().unwrap();
    let candidate = CapNode::new(
        id,
        Rights::READ,
        Arc::clone(&object),
        ObjectType::Resource,
        None,
        permit,
    );
    assert!(Arc::try_new_in(candidate, FailingAllocator).is_err());
    assert_eq!(
        TEST.live_count(),
        0,
        "allocation failure leaked a reservation"
    );
    assert_eq!(
        Arc::strong_count(&object),
        1,
        "allocation failure retained backing"
    );
}
