#![allow(dead_code)]
#[path = "../boot/uefi/src/abi.rs"]
mod abi;
#[path = "../boot/uefi/src/acpi.rs"]
mod acpi_snapshot;
#[path = "../boot/uefi/src/paging.rs"]
mod paging;
#[path = "../boot/uefi/src/payload.rs"]
mod payload;
#[path = "../boot/uefi/src/uefi.rs"]
mod uefi;
const PAGE_SIZE: u64 = 4096;
struct Segment {
    start: u64,
    end: u64,
    flags: u32,
}
struct KernelImage {
    start: u64,
    end: u64,
    segments: [Segment; 16],
    segment_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        alloc::{alloc, dealloc, Layout},
        cell::RefCell,
    };
    struct State {
        length: u64,
        position: usize,
        read_mode: u8,
        open_status: usize,
        alloc_fail: bool,
        allocated: usize,
        freed: usize,
        closed: usize,
        close_status: usize,
    }
    thread_local! { static STATE: RefCell<State> = const { RefCell::new(State {
        length: 5000, position: 0, read_mode: 0, open_status: 0,
        alloc_fail: false, allocated: 0, freed: 0, closed: 0, close_status: 0,
    }) }; }
    extern "efiapi" fn open(
        root: *mut uefi::FileProtocol,
        out: *mut *mut uefi::FileProtocol,
        _: *const u16,
        _: u64,
        _: u64,
    ) -> usize {
        let status = STATE.with(|s| s.borrow().open_status);
        if status == 0 {
            unsafe {
                *out = root;
            }
        }
        status
    }
    extern "efiapi" fn close(_: *mut uefi::FileProtocol) -> usize {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.closed += 1;
            s.close_status
        })
    }
    extern "efiapi" fn seek(_: *mut uefi::FileProtocol, position: u64) -> usize {
        if position == 0 {
            STATE.with(|s| s.borrow_mut().position = 0);
        }
        0
    }
    extern "efiapi" fn position(_: *mut uefi::FileProtocol, out: *mut u64) -> usize {
        unsafe {
            *out = STATE.with(|s| s.borrow().length);
        }
        0
    }
    extern "efiapi" fn read(
        _: *mut uefi::FileProtocol,
        size: *mut usize,
        buffer: *mut u8,
    ) -> usize {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            match s.read_mode {
                1 if s.position != 0 => return uefi::LOAD_ERROR,
                2 => {
                    unsafe {
                        *size = 0;
                    }
                    return 0;
                }
                3 => {
                    unsafe {
                        *size += 1;
                    }
                    return 0;
                }
                _ => {}
            }
            let count = unsafe { *size }.min(17);
            for index in 0..count {
                unsafe {
                    *buffer.add(index) = (s.position + index) as u8;
                }
            }
            s.position += count;
            unsafe {
                *size = count;
            }
            0
        })
    }
    extern "efiapi" fn allocate(_: u32, _: u32, pages: usize, out: *mut u64) -> usize {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if s.alloc_fail {
                return uefi::OUT_OF_RESOURCES;
            }
            let layout = Layout::from_size_align(pages * 4096, 4096).unwrap();
            let buffer = unsafe { alloc(layout) };
            assert!(!buffer.is_null());
            unsafe {
                *out = buffer as u64;
            }
            s.allocated += 1;
            0
        })
    }
    pub(crate) extern "efiapi" fn free(address: u64, pages: usize) -> usize {
        unsafe {
            dealloc(
                address as *mut u8,
                Layout::from_size_align(pages * 4096, 4096).unwrap(),
            );
        }
        STATE.with(|s| s.borrow_mut().freed += 1);
        0
    }
    pub(crate) fn services() -> uefi::BootServices {
        let mut result: uefi::BootServices = unsafe { std::mem::zeroed() };
        result.functions[2] = allocate as *const () as usize;
        result.functions[3] = free as *const () as usize;
        result
    }
    fn file() -> uefi::FileProtocol {
        uefi::FileProtocol {
            revision: 0,
            open,
            close,
            delete: 0,
            read,
            write: 0,
            get_position: position,
            set_position: seek,
        }
    }
    #[test]
    fn exact_chunked_read_zeroes_padding_and_retains_source() {
        let mut services = services();
        let mut file = file();
        let result = unsafe { payload::read_boot_payload(&mut services, &mut file) }.unwrap();
        assert_eq!(result.byte_len, 5000);
        let bytes = unsafe { std::slice::from_raw_parts(result.physical_start as *const u8, 8192) };
        for (index, value) in bytes[..5000].iter().enumerate() {
            assert_eq!(*value, index as u8);
        }
        assert!(bytes[5000..].iter().all(|b| *b == 0));
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!((s.allocated, s.freed, s.closed), (1, 0, 1));
        });
        free(result.physical_start, 2);
    }
    #[test]
    fn failed_short_zero_and_oversized_reads_free_the_allocation() {
        for mode in 1..=3 {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.read_mode = mode;
                s.position = 0;
            });
            let mut services = services();
            let mut file = file();
            assert_eq!(
                unsafe { payload::read_boot_payload(&mut services, &mut file) },
                Err(uefi::LOAD_ERROR)
            );
            STATE.with(|s| {
                let s = s.borrow();
                assert_eq!(s.allocated, s.freed);
                assert_eq!(s.closed, mode as usize);
            });
        }
    }
    #[test]
    fn missing_is_optional_other_open_errors_are_not() {
        let mut services = services();
        let mut file = file();
        STATE.with(|s| s.borrow_mut().open_status = uefi::NOT_FOUND);
        assert_eq!(
            unsafe { payload::read_boot_payload(&mut services, &mut file) },
            Ok(abi::BootPayload::NONE)
        );
        STATE.with(|s| s.borrow_mut().open_status = uefi::LOAD_ERROR);
        assert_eq!(
            unsafe { payload::read_boot_payload(&mut services, &mut file) },
            Err(uefi::LOAD_ERROR)
        );
        STATE.with(|s| assert_eq!(s.borrow().allocated, 0));
    }
    #[test]
    fn close_failure_releases_successful_source() {
        STATE.with(|s| s.borrow_mut().close_status = uefi::LOAD_ERROR);
        let mut services = services();
        let mut file = file();
        assert_eq!(
            unsafe { payload::read_boot_payload(&mut services, &mut file) },
            Err(uefi::LOAD_ERROR)
        );
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!((s.allocated, s.freed, s.closed), (1, 1, 1));
        });
    }
    #[test]
    fn downstream_failure_frees_payload_but_exit_attempt_and_handoff_retain_it() {
        use std::sync::atomic::{AtomicBool, Ordering};
        for state in 0..3 {
            let mut services = services();
            let mut file = file();
            let source = unsafe { payload::read_boot_payload(&mut services, &mut file) }.unwrap();
            let attempted = AtomicBool::new(false);
            let mut owner =
                unsafe { payload::PayloadOwner::new(&mut services, source, &attempted) };
            match state {
                1 => attempted.store(true, Ordering::Relaxed),
                2 => owner.disarm(),
                _ => {}
            }
            drop(owner);
            STATE.with(|s| assert_eq!(s.borrow().freed, usize::from(state == 0)));
            if state != 0 {
                free(source.physical_start, 2);
            }
            // Counts accumulate in this thread across the three independently owned sources.
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.allocated = 0;
                s.freed = 0;
            });
        }
    }
    #[test]
    fn invalid_lengths_and_allocation_failure_never_read() {
        for length in [0, abi::payload::MAX_PAYLOAD_BYTES + 1, u64::MAX] {
            STATE.with(|s| s.borrow_mut().length = length);
            let mut services = services();
            let mut file = file();
            assert_eq!(
                unsafe { payload::read_boot_payload(&mut services, &mut file) },
                Err(uefi::LOAD_ERROR)
            );
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.length = 5000;
            s.alloc_fail = true;
        });
        let mut services = services();
        let mut file = file();
        assert_eq!(
            unsafe { payload::read_boot_payload(&mut services, &mut file) },
            Err(uefi::OUT_OF_RESOURCES)
        );
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!((s.allocated, s.position, s.closed), (0, 0, 4));
        });
    }
}
