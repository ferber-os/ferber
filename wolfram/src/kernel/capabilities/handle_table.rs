use super::{CapNode, Rights};
use alloc::sync::Arc;

/// Fixed-width capability metadata; no kernel addresses cross the ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CapInfo {
    pub object_type: u32,
    pub rights: u32,
}

const _: () = assert!(core::mem::size_of::<CapInfo>() == 8);

impl CapInfo {
    pub fn to_le_bytes(self) -> [u8; 8] {
        let mut bytes = [0; 8];
        bytes[..4].copy_from_slice(&self.object_type.to_le_bytes());
        bytes[4..].copy_from_slice(&self.rights.to_le_bytes());
        bytes
    }
}

const MAX_HANDLES: usize = 256;

/// Per-process handle table. Maps `u32` indices to owning capability references.
///
/// Processes are born with an empty table. Capabilities are explicitly
/// passed in at spawn time. No ambient authority.
pub struct HandleTable {
    /// Sparse array of capability nodes. `None` = slot is free.
    /// Phase 2 uses a fixed-size array. Phase 3 can grow dynamically.
    entries: [Option<Arc<CapNode>>; MAX_HANDLES],
    /// Number of live entries (for diagnostics and `fer cap audit`).
    count: u32,
}

impl HandleTable {
    pub fn new() -> Self {
        Self {
            entries: [const { None }; MAX_HANDLES],
            count: 0,
        }
    }

    /// Insert a CapNode, return the assigned handle index.
    pub fn insert(&mut self, node: Arc<CapNode>) -> Result<u32, (CapError, Arc<CapNode>)> {
        if self.count as usize >= MAX_HANDLES {
            return Err((CapError::TableFull, node));
        }

        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.is_none() {
                *entry = Some(node);
                self.count += 1;
                return Ok(i as u32);
            }
        }

        Err((CapError::TableFull, node))
    }

    /// Look up a handle. Returns the CapNode if the handle is valid and the
    /// node hasn't been revoked.
    pub fn lookup(&self, handle: u32) -> Result<&Arc<CapNode>, CapError> {
        let entry = self
            .entries
            .get(handle as usize)
            .and_then(Option::as_ref)
            .ok_or(CapError::InvalidHandle)?;

        let node = entry;
        if !node.is_valid() {
            return Err(CapError::Revoked);
        }

        Ok(node)
    }

    /// Remove a handle (used on close or transfer-out).
    pub fn remove(&mut self, handle: u32) -> Result<Arc<CapNode>, CapError> {
        if let Some(entry) = self.entries.get_mut(handle as usize) {
            if let Some(node) = entry.take() {
                self.count -= 1;
                return Ok(node);
            }
        }
        Err(CapError::InvalidHandle)
    }

    pub fn is_full(&self) -> bool {
        self.count as usize == MAX_HANDLES
    }

    /// Return the number of live handles in the table.
    pub fn count(&self) -> usize {
        self.count as usize
    }

    /// Duplicate the same capability node and rights without allocating a node.
    pub fn duplicate(&mut self, handle: u32) -> Result<u32, CapError> {
        let node = self.lookup(handle)?;
        if !node.rights().contains(Rights::DUPLICATE) {
            return Err(CapError::InsufficientRights);
        }
        let node = Arc::clone(node);
        self.insert(node).map_err(|(error, _node)| error)
    }

    pub fn inspect(&self, handle: u32) -> Result<CapInfo, CapError> {
        let node = self.lookup(handle)?;
        if !node.rights().contains(Rights::INSPECT) {
            return Err(CapError::InsufficientRights);
        }
        Ok(CapInfo {
            object_type: node.object_type() as u32,
            rights: node.rights().bits(),
        })
    }

    /// Extract a capability node for transfer out of this process.
    /// Fails if the handle is invalid, revoked, or lacks TRANSFER rights.
    pub fn transfer_send(&mut self, handle: u32) -> Result<Arc<CapNode>, CapError> {
        let node = self.lookup(handle)?;
        if !node.rights().contains(super::Rights::TRANSFER) {
            return Err(CapError::InsufficientRights);
        }
        self.remove(handle)
    }

    /// Insert a transferred capability node into this process's handle table.
    pub fn transfer_recv(&mut self, node: Arc<CapNode>) -> Result<u32, (CapError, Arc<CapNode>)> {
        self.insert(node)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapError {
    InvalidHandle,
    Revoked,
    TableFull,
    InsufficientRights,
    InvalidRights,
    OutOfMemory,
    NodeCapacity,
}
