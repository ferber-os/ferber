use crate::{abi, uefi, PAGE_SIZE};
use core::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};
use uefi::{BootServices, Status};

pub unsafe fn read_boot_payload(
    services: *mut BootServices,
    root: *mut uefi::FileProtocol,
) -> Result<abi::BootPayload, Status> {
    const PATH: [u16; 10] = [92, 66, 79, 79, 84, 46, 69, 76, 70, 0];
    let mut file = ptr::null_mut();
    let status = ((*root).open)(root, &mut file, PATH.as_ptr(), uefi::FILE_MODE_READ, 0);
    if status == uefi::NOT_FOUND {
        return Ok(abi::BootPayload::NONE);
    }
    if status != uefi::SUCCESS {
        return Err(status);
    }
    if file.is_null() {
        return Err(uefi::LOAD_ERROR);
    }
    let result = read_payload_pages(services, file);
    let close_status = ((*file).close)(file);
    if close_status != uefi::SUCCESS {
        if let Ok(payload) = result {
            (*services).free_pages(
                payload.physical_start,
                payload.byte_len.div_ceil(PAGE_SIZE) as usize,
            );
        }
        return Err(close_status);
    }
    result
}

unsafe fn read_payload_pages(
    services: *mut BootServices,
    file: *mut uefi::FileProtocol,
) -> Result<abi::BootPayload, Status> {
    let status = ((*file).set_position)(file, u64::MAX);
    if status != uefi::SUCCESS {
        return Err(status);
    }
    let mut length = 0;
    let status = ((*file).get_position)(file, &mut length);
    if status != uefi::SUCCESS {
        return Err(status);
    }
    if length == 0 || length > abi::payload::MAX_PAYLOAD_BYTES {
        return Err(uefi::LOAD_ERROR);
    }
    let status = ((*file).set_position)(file, 0);
    if status != uefi::SUCCESS {
        return Err(status);
    }
    let pages = length.div_ceil(PAGE_SIZE) as usize;
    let mut address = 0;
    let status = (*services).allocate_pages(uefi::EFI_LOADER_DATA, pages, &mut address);
    if status != uefi::SUCCESS {
        return Err(status);
    }
    let payload = match abi::BootPayload::new(address, length) {
        Ok(value) => value,
        Err(_) => {
            (*services).free_pages(address, pages);
            return Err(uefi::LOAD_ERROR);
        }
    };
    ptr::write_bytes(address as *mut u8, 0, pages * PAGE_SIZE as usize);
    let mut total = 0;
    while total < length as usize {
        let remaining = length as usize - total;
        let mut chunk = remaining;
        let status = ((*file).read)(file, &mut chunk, (address as *mut u8).add(total));
        if status != uefi::SUCCESS || chunk == 0 || chunk > remaining {
            (*services).free_pages(address, pages);
            return Err(if status == uefi::SUCCESS {
                uefi::LOAD_ERROR
            } else {
                status
            });
        }
        total += chunk;
    }
    Ok(payload)
}

/// Owns only the new payload allocation until firmware services become unavailable.
pub struct PayloadOwner<'a> {
    services: *mut BootServices,
    payload: abi::BootPayload,
    exit_attempted: &'a AtomicBool,
    armed: bool,
}
impl<'a> PayloadOwner<'a> {
    pub unsafe fn new(
        services: *mut BootServices,
        payload: abi::BootPayload,
        exit_attempted: &'a AtomicBool,
    ) -> Self {
        Self {
            services,
            payload,
            exit_attempted,
            armed: true,
        }
    }
    pub fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for PayloadOwner<'_> {
    fn drop(&mut self) {
        if self.armed && self.payload.byte_len != 0 && !self.exit_attempted.load(Ordering::Relaxed)
        {
            unsafe {
                (*self.services).free_pages(
                    self.payload.physical_start,
                    self.payload.byte_len.div_ceil(PAGE_SIZE) as usize,
                );
            }
        }
    }
}
