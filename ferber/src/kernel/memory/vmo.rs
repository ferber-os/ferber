//! Virtual Memory Object (VMO) — Ferber's fundamental memory primitive.
//! Every region of virtual memory is backed by a VMO.
//! No anonymous memory. No implicit backing. Ever.

#![allow(dead_code)]

use alloc::vec::Vec;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::kernel::sync::SpinLock;

use crate::kernel::capabilities::{KernelObject, Rights};
use crate::kernel::memory::paging::MapFlags;
use crate::kernel::memory::{bitmap, phys_map};

const PAGE_SIZE: usize = 4096;
pub const MAX_USER_VMO_SIZE: usize = 16 * 1024 * 1024;
static USER_LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);
static USER_USED: AtomicUsize = AtomicUsize::new(0);

pub const fn user_budget_limit(managed_pages: usize) -> usize {
    let quarter = managed_pages / 4;
    if quarter < 16_384 { quarter } else { 16_384 }
}

pub fn init_user_budget(managed_pages: usize) {
    assert!(USER_LIMIT.compare_exchange(usize::MAX, user_budget_limit(managed_pages),
        Ordering::Release, Ordering::Relaxed).is_ok(), "user VMO budget initialized twice");
}

pub fn user_budget_stats() -> (usize, usize) {
    (USER_LIMIT.load(Ordering::Acquire), USER_USED.load(Ordering::Acquire))
}

struct UserCharge(usize);

impl UserCharge {
    fn reserve(pages: usize) -> Result<Self, VmoError> {
        let limit = USER_LIMIT.load(Ordering::Acquire);
        assert_ne!(limit, usize::MAX, "user VMO budget not initialized");
        USER_USED.try_update(Ordering::AcqRel, Ordering::Acquire,
            |used| used.checked_add(pages).filter(|next| *next <= limit))
            .map_err(|_| VmoError::UserBudget)?;
        Ok(Self(pages))
    }
}

impl Drop for UserCharge {
    fn drop(&mut self) {
        USER_USED.fetch_sub(self.0, Ordering::AcqRel);
    }
}

/// Errors returned by VMO operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmoError {
    /// Requested size is zero.
    ZeroSize,
    TooLarge,
    UserBudget,
    InvalidBacking,
    /// Physical allocator ran out of frames.
    OutOfMemory,
    /// Offset and length exceed the VMO size.
    OutOfBounds,
    /// Operation denied due to missing capability rights.
    PermissionDenied,
    /// Physical frame address is unaligned.
    InvalidAlignment,
}

/// A container of physical frames allocated to back a VMO.
#[derive(Debug)]
pub struct VmoFrames {
    frames: FrameStorage,
}

#[derive(Debug)]
enum FrameStorage {
    Scattered(Vec<usize>),
    Contiguous { base: usize, count: usize },
}

impl VmoFrames {
    pub fn new(frames: Vec<usize>) -> Self {
        Self {
            frames: FrameStorage::Scattered(frames),
        }
    }
    pub fn contiguous(base: usize, count: usize) -> Self {
        Self {
            frames: FrameStorage::Contiguous { base, count },
        }
    }
    pub fn count(&self) -> usize {
        match &self.frames {
            FrameStorage::Scattered(frames) => frames.len(),
            FrameStorage::Contiguous { count, .. } => *count,
        }
    }
    pub fn get(&self, index: usize) -> Option<usize> {
        match &self.frames {
            FrameStorage::Scattered(frames) => frames.get(index).copied(),
            FrameStorage::Contiguous { base, count } => {
                (index < *count).then(|| base + index * PAGE_SIZE)
            }
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.count()).map(|index| self.get(index).unwrap())
    }
}

/// A Virtual Memory Object.
/// Represents a contiguous sequence of bytes backed by physical pages.
pub struct Vmo {
    size: usize,
    frames: VmoFrames,
    is_physical: bool,
    content: SpinLock<()>,
    user_charge: Option<UserCharge>,
}

unsafe impl Send for Vmo {}
unsafe impl Sync for Vmo {}

impl KernelObject for Vmo {
    fn type_name(&self) -> &'static str {
        "Vmo"
    }
}

impl Vmo {
    pub fn create_user(size: usize) -> Result<Self, VmoError> {
        if size == 0 { return Err(VmoError::ZeroSize); }
        if size > MAX_USER_VMO_SIZE { return Err(VmoError::TooLarge); }
        let charge = UserCharge::reserve(size.div_ceil(PAGE_SIZE))?;
        let mut vmo = Self::create(size)?;
        vmo.user_charge = Some(charge);
        Ok(vmo)
    }

    pub fn is_owned_ram(&self) -> bool { !self.is_physical }

    pub fn validate_owned_range(&self, offset: usize, length: usize) -> Result<(), VmoError> {
        if !self.is_owned_ram() { return Err(VmoError::InvalidBacking); }
        let end = offset.checked_add(length).filter(|end| *end <= self.size)
            .ok_or(VmoError::OutOfBounds)?;
        if length == 0 { return Ok(()); }
        for index in offset / PAGE_SIZE..end.div_ceil(PAGE_SIZE) {
            let frame = self.frames.get(index).ok_or(VmoError::OutOfBounds)?;
            if phys_map::mmio_to_virt(frame).is_some() || phys_map::phys_to_virt(frame).is_none() {
                return Err(VmoError::InvalidBacking);
            }
        }
        Ok(())
    }

    /// Caller supplies a kernel buffer disjoint from this VMO's physical backing.
    pub fn read_owned(&self, offset: usize, buffer: &mut [u8]) -> Result<(), VmoError> {
        let _content = self.content.lock();
        self.validate_owned_range(offset, buffer.len())?;
        self.copy_owned(offset, buffer.as_mut_ptr(), buffer.len(), false);
        Ok(())
    }

    /// Caller supplies a kernel snapshot disjoint from this VMO's physical backing.
    pub fn write_owned(&self, offset: usize, buffer: &[u8]) -> Result<(), VmoError> {
        let _content = self.content.lock();
        self.validate_owned_range(offset, buffer.len())?;
        self.copy_owned(offset, buffer.as_ptr().cast_mut(), buffer.len(), true);
        Ok(())
    }

    fn copy_owned(&self, offset: usize, buffer: *mut u8, length: usize, write: bool) {
        let mut copied = 0;
        while copied < length {
            let position = offset + copied;
            let count = (PAGE_SIZE - position % PAGE_SIZE).min(length - copied);
            let frame = self.frames.get(position / PAGE_SIZE).expect("validated VMO frame");
            let alias = phys_map::phys_to_virt(frame).expect("validated VMO RAM alias");
            unsafe {
                let backing = (alias + position % PAGE_SIZE) as *mut u8;
                if write { ptr::copy_nonoverlapping(buffer.add(copied), backing, count); }
                else { ptr::copy_nonoverlapping(backing, buffer.add(copied), count); }
            }
            copied += count;
        }
    }

    /// Create a new dynamically allocated VMO with `size` bytes.
    /// Allocates physical frames from the frame bitmap allocator.
    pub fn create(size: usize) -> Result<Self, VmoError> {
        if size == 0 {
            return Err(VmoError::ZeroSize);
        }

        let num_pages = size.div_ceil(PAGE_SIZE);
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(num_pages)
            .map_err(|_| VmoError::OutOfMemory)?;

        for _ in 0..num_pages {
            match bitmap::allocate_frame() {
                Some(frame) => {
                    // Clear the complete frame, including bytes beyond the VMO's logical end.
                    unsafe {
                        ptr::write_bytes(
                            phys_map::phys_to_virt(frame).expect("VMO RAM alias") as *mut u8,
                            0,
                            PAGE_SIZE,
                        )
                    };
                    frames.push(frame);
                }
                None => {
                    for frame in frames {
                        let _ = bitmap::free_frame(frame);
                    }
                    return Err(VmoError::OutOfMemory);
                }
            }
        }

        Ok(Self {
            size,
            frames: VmoFrames::new(frames),
            is_physical: false,
            content: SpinLock::new(()),
            user_charge: None,
        })
    }

    /// Create a contiguous dynamically allocated VMO.
    pub fn create_contiguous(size: usize) -> Result<Self, VmoError> {
        if size == 0 {
            return Err(VmoError::ZeroSize);
        }

        let num_pages = size.div_ceil(PAGE_SIZE);
        let base = bitmap::allocate_frames(num_pages).ok_or(VmoError::OutOfMemory)?;
        unsafe {
            ptr::write_bytes(
                phys_map::phys_to_virt(base).expect("VMO RAM alias") as *mut u8,
                0,
                num_pages * PAGE_SIZE,
            )
        };

        Ok(Self {
            size,
            frames: VmoFrames::contiguous(base, num_pages),
            is_physical: false,
            content: SpinLock::new(()),
            user_charge: None,
        })
    }

    /// Create a physical VMO for MMIO / hardware memory mappings.
    /// Does not allocate from the frame pool and will not free frames on drop.
    pub fn create_physical(base: usize, size: usize) -> Result<Self, VmoError> {
        if size == 0 {
            return Err(VmoError::ZeroSize);
        }
        if !base.is_multiple_of(PAGE_SIZE) {
            return Err(VmoError::InvalidAlignment);
        }

        let num_pages = size.div_ceil(PAGE_SIZE);
        num_pages.checked_mul(PAGE_SIZE).and_then(|bytes| base.checked_add(bytes))
            .ok_or(VmoError::OutOfBounds)?;
        Ok(Self {
            size,
            frames: VmoFrames::contiguous(base, num_pages),
            is_physical: true,
            content: SpinLock::new(()),
            user_charge: None,
        })
    }

    /// Total size of this VMO in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Number of 4 KiB frames backing this VMO.
    pub fn page_count(&self) -> usize {
        self.frames.count()
    }

    /// Access the backing frames list.
    pub fn frames(&self) -> &VmoFrames {
        &self.frames
    }

    /// Check if capability rights permit mapping this VMO with the requested `MapFlags`.
    pub fn check_mapping_rights(rights: Rights, flags: MapFlags) -> Result<(), VmoError> {
        if !rights.contains(Rights::MAP) {
            return Err(VmoError::PermissionDenied);
        }
        if flags.readable && !rights.contains(Rights::READ) {
            return Err(VmoError::PermissionDenied);
        }
        if flags.writable && !rights.contains(Rights::WRITE) {
            return Err(VmoError::PermissionDenied);
        }
        if flags.executable && !rights.contains(Rights::EXECUTE) {
            return Err(VmoError::PermissionDenied);
        }
        Ok(())
    }

    /// Read data from the VMO at `offset` into `buffer`.
    pub fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<(), VmoError> {
        let _content = self.content.lock();
        if offset
            .checked_add(buffer.len())
            .is_none_or(|end| end > self.size)
        {
            return Err(VmoError::OutOfBounds);
        }

        let mut bytes_read = 0;
        while bytes_read < buffer.len() {
            let current_offset = offset + bytes_read;
            let page_idx = current_offset / PAGE_SIZE;
            let page_offset = current_offset % PAGE_SIZE;
            let to_read = core::cmp::min(PAGE_SIZE - page_offset, buffer.len() - bytes_read);

            let frame = self.frames.get(page_idx).ok_or(VmoError::OutOfBounds)?;
            if let Some(device) = phys_map::mmio_to_virt(frame) {
                for byte in 0..to_read {
                    buffer[bytes_read + byte] = unsafe {
                        ptr::read_volatile((device + page_offset + byte) as *const u8)
                    };
                }
                bytes_read += to_read;
                continue;
            }
            unsafe {
                let src = (phys_map::phys_to_virt(frame).ok_or(VmoError::OutOfBounds)?
                    as *const u8)
                    .add(page_offset);
                let dst = buffer.as_mut_ptr().add(bytes_read);
                ptr::copy_nonoverlapping(src, dst, to_read);
            }

            bytes_read += to_read;
        }

        Ok(())
    }

    /// Write data to the VMO at `offset` from `buffer`.
    pub fn write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), VmoError> {
        self.write_shared(offset, buffer)
    }

    fn write_shared(&self, offset: usize, buffer: &[u8]) -> Result<(), VmoError> {
        let _content = self.content.lock();
        if offset
            .checked_add(buffer.len())
            .is_none_or(|end| end > self.size)
        {
            return Err(VmoError::OutOfBounds);
        }

        let mut bytes_written = 0;
        while bytes_written < buffer.len() {
            let current_offset = offset + bytes_written;
            let page_idx = current_offset / PAGE_SIZE;
            let page_offset = current_offset % PAGE_SIZE;
            let to_write = core::cmp::min(PAGE_SIZE - page_offset, buffer.len() - bytes_written);

            let frame = self.frames.get(page_idx).ok_or(VmoError::OutOfBounds)?;
            if let Some(device) = phys_map::mmio_to_virt(frame) {
                for byte in 0..to_write {
                    unsafe { ptr::write_volatile((device + page_offset + byte) as *mut u8,
                        buffer[bytes_written + byte]) };
                }
                bytes_written += to_write;
                continue;
            }
            unsafe {
                let src = buffer.as_ptr().add(bytes_written);
                let dst = (phys_map::phys_to_virt(frame).ok_or(VmoError::OutOfBounds)? as *mut u8)
                    .add(page_offset);
                ptr::copy_nonoverlapping(src, dst, to_write);
            }

            bytes_written += to_write;
        }

        Ok(())
    }
}

impl Drop for Vmo {
    fn drop(&mut self) {
        if !self.is_physical {
            for frame in self.frames.iter() {
                let _ = bitmap::free_frame(frame);
            }
        }
    }
}
