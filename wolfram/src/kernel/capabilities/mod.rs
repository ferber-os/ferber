//! Wolfram Capability System
//!
//! This is the core of the kernel. Everything else exists to serve this.
//!
//! If you're reading this, you're either debugging something horrible
//! or you're curious. Either way, welcome.

#![allow(dead_code)]
#![allow(unused_imports)]

pub mod allocator;
pub mod handle_table;

pub use allocator::{CapNodeAllocator, CAP_NODE_ALLOCATOR, NODE_CAPACITY};
pub use handle_table::{CapError, HandleTable};

use alloc::sync::Arc;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Rights: u32 {
        const DUPLICATE = 1 << 0;
        const TRANSFER  = 1 << 1;
        const READ      = 1 << 2;
        const WRITE     = 1 << 3;
        const EXECUTE   = 1 << 4;
        const MAP       = 1 << 5;
        const SIGNAL    = 1 << 6;
        const WAIT      = 1 << 7;
        const INSPECT   = 1 << 8;
        const MANAGE    = 1 << 9;
    }
}

/// Marker trait for kernel-managed objects that can be held via handles.
pub trait KernelObject: Send + Sync {
    /// Human-readable type name for diagnostics and `fer cap audit`.
    fn type_name(&self) -> &'static str;
}

/// Runtime type tag for kernel objects. Serialisable, matchable in syscall dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectType {
    Process = 0,
    Thread = 1,
    Vmo = 2,
    Channel = 3,
    Job = 4,
    Resource = 5,
}

pub struct Process;
impl KernelObject for Process {
    fn type_name(&self) -> &'static str {
        "Process"
    }
}

pub struct Thread;
impl KernelObject for Thread {
    fn type_name(&self) -> &'static str {
        "Thread"
    }
}

pub use crate::kernel::memory::vmo::Vmo;

pub struct Channel;
impl KernelObject for Channel {
    fn type_name(&self) -> &'static str {
        "Channel"
    }
}

pub struct Job;
impl KernelObject for Job {
    fn type_name(&self) -> &'static str {
        "Job"
    }
}

pub struct Resource;
impl KernelObject for Resource {
    fn type_name(&self) -> &'static str {
        "Resource"
    }
}

/// Marker traits for compile-time rights enforcement.
/// Each right type is an uninhabited ZST.
pub trait RightMarker {}
pub struct Read;
impl RightMarker for Read {}
pub struct Write;
impl RightMarker for Write {}
pub struct ReadWrite;
impl RightMarker for ReadWrite {}
pub struct Execute;
impl RightMarker for Execute {}

/// A typed, rights-annotated handle to a kernel object.
///
/// `T` is the kernel object type (must impl `KernelObject`).
/// `R` is a rights marker type for compile-time enforcement.
///
/// The handle also owns a CapNode reference for runtime enforcement
/// at the syscall boundary.
pub struct Handle<T: KernelObject, R: RightMarker> {
    /// Index into the owning process's handle table.
    raw: u32,
    /// Owning reference to the capability node backing this handle.
    node: Arc<CapNode>,
    _type: PhantomData<T>,
    _rights: PhantomData<R>,
}

impl<T: KernelObject, R: RightMarker> Handle<T, R> {
    pub fn new(raw: u32, node: Arc<CapNode>) -> Self {
        Self {
            raw,
            node,
            _type: PhantomData,
            _rights: PhantomData,
        }
    }

    pub fn raw(&self) -> u32 {
        self.raw
    }

    pub fn node(&self) -> &Arc<CapNode> {
        &self.node
    }

    /// Create an attenuated capability node from this handle's node.
    /// The caller is responsible for inserting the new node into a HandleTable.
    pub fn attenuate<NewR: RightMarker>(
        &self,
        new_rights: Rights,
    ) -> Result<Handle<T, NewR>, CapError> {
        let child = self.node().attenuate(new_rights)?;
        Ok(Handle {
            raw: u32::MAX,
            node: child,
            _type: PhantomData,
            _rights: PhantomData,
        })
    }
}

/// A capability node — the indirection layer between handles and kernel objects.
///
/// Handle → CapNode → Object.
/// Revoking a CapNode kills every handle that points through it.
pub struct CapNode {
    id: u64,
    valid: AtomicBool,
    rights: Rights,
    object: Arc<dyn KernelObject>,
    object_type: ObjectType,
    parent: Option<Arc<CapNode>>,
    _permit: allocator::NodePermit,
}

impl core::fmt::Debug for CapNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CapNode")
            .field("id", &self.id)
            .field("rights", &self.rights)
            .field("object_type", &self.object_type)
            .finish_non_exhaustive()
    }
}

impl CapNode {
    fn new(
        id: u64,
        rights: Rights,
        object: Arc<dyn KernelObject>,
        object_type: ObjectType,
        parent: Option<Arc<CapNode>>,
        permit: allocator::NodePermit,
    ) -> Self {
        Self {
            id,
            valid: AtomicBool::new(true),
            rights,
            object,
            object_type,
            parent,
            _permit: permit,
        }
    }

    /// Invalidate this node and every descendant through their ancestor checks.
    pub fn revoke(&self) {
        self.valid.store(false, Ordering::Release);
    }

    /// Immutable strong parent links keep this entire walk alive without a lock.
    pub fn is_valid(&self) -> bool {
        let mut node = self;
        loop {
            if !node.valid.load(Ordering::Acquire) {
                return false;
            }
            match node.parent.as_deref() {
                Some(parent) => node = parent,
                None => return true,
            }
        }
    }

    pub fn rights(&self) -> Rights {
        self.rights
    }
    pub fn object_type(&self) -> ObjectType {
        self.object_type
    }
    pub fn parent(&self) -> Option<&Arc<CapNode>> {
        self.parent.as_ref()
    }

    /// Derive a distinct child while retaining the source capability.
    pub fn attenuate(self: &Arc<Self>, new_rights: Rights) -> Result<Arc<Self>, CapError> {
        if !self.is_valid() {
            return Err(CapError::Revoked);
        }
        if !self.rights.contains(Rights::DUPLICATE) {
            return Err(CapError::InsufficientRights);
        }
        if Rights::from_bits(new_rights.bits()).is_none() {
            return Err(CapError::InvalidRights);
        }
        if !self.rights.contains(new_rights) {
            return Err(CapError::InsufficientRights);
        }
        CAP_NODE_ALLOCATOR.alloc_child(self, new_rights)
    }
}

impl Drop for CapNode {
    fn drop(&mut self) {
        let mut parent = self.parent.take();
        while let Some(node) = parent {
            parent = Arc::into_inner(node).and_then(|mut node| node.parent.take());
        }
    }
}

pub fn self_test() {
    allocator::self_test();
}
