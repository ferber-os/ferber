//! Read firmware-assigned BAR bounds without probing an active controller.
use crate::{
    abi::pci_resource::{self, ControllerResource},
    uefi::{self, BootServices, Guid, Handle, Status},
};
use core::{ffi::c_void, ptr};
const PCI_IO: Guid = Guid {
    data1: 0x4cf5b200,
    data2: 0x68b8,
    data3: 0x4ca5,
    data4: [0x9e, 0xec, 0xb2, 0x3e, 0x3f, 0x50, 0x02, 0x9a],
};
#[repr(C)]
struct PciIo {
    functions: [usize; 18],
    rom_size: u64,
    rom_image: *mut c_void,
}
impl PciIo {
    unsafe fn read(&mut self, offset: u32, output: &mut [u32]) -> Result<(), Status> {
        let f: extern "efiapi" fn(*mut Self, u32, u32, usize, *mut c_void) -> Status =
            core::mem::transmute(self.functions[6]);
        status(f(self, 2, offset, output.len(), output.as_mut_ptr().cast()))
    }
    unsafe fn location(&mut self) -> Result<(u16, u8, u8, u8), Status> {
        let f: extern "efiapi" fn(
            *mut Self,
            *mut usize,
            *mut usize,
            *mut usize,
            *mut usize,
        ) -> Status = core::mem::transmute(self.functions[14]);
        let (mut segment, mut bus, mut slot, mut function) = (0, 0, 0, 0);
        status(f(self, &mut segment, &mut bus, &mut slot, &mut function))?;
        if segment > u16::MAX as usize || bus > 255 || slot > 31 || function > 7 {
            return Err(uefi::LOAD_ERROR);
        }
        Ok((segment as u16, bus as u8, slot as u8, function as u8))
    }
    unsafe fn resource(
        &mut self,
        services: &mut BootServices,
        raw: u64,
    ) -> Result<(u64, u64), Status> {
        let f: extern "efiapi" fn(*mut Self, u8, *mut u64, *mut *mut c_void) -> Status =
            core::mem::transmute(self.functions[16]);
        let mut resources = ptr::null_mut();
        let result = f(self, 0, ptr::null_mut(), &mut resources);
        if result != uefi::SUCCESS {
            return Err(result);
        }
        if resources.is_null() {
            return Err(uefi::LOAD_ERROR);
        }
        // The firmware protocol owns the returned allocation and descriptor contract.
        let head = core::slice::from_raw_parts(resources.cast::<u8>(), 3);
        let parsed = if head == [0x8a, 43, 0] {
            pci_resource::descriptor(core::slice::from_raw_parts(resources.cast::<u8>(), 48), raw)
                .map_err(|_| uefi::LOAD_ERROR)
        } else {
            Err(uefi::LOAD_ERROR)
        };
        status(services.free_pool(resources.cast()))?;
        parsed
    }
}
fn status(value: Status) -> Result<(), Status> {
    if value == uefi::SUCCESS {
        Ok(())
    } else {
        Err(value)
    }
}
/// # Safety
/// Called before ExitBootServices with exclusive firmware protocol access.
pub unsafe fn discover(services: *mut BootServices) -> Result<ControllerResource, Status> {
    let services = &mut *services;
    let locate: extern "efiapi" fn(
        u32,
        *const Guid,
        *mut c_void,
        *mut usize,
        *mut *mut Handle,
    ) -> Status = core::mem::transmute(services.functions[36]);
    let mut count = 0;
    let mut handles = ptr::null_mut();
    status(locate(
        2,
        &PCI_IO,
        ptr::null_mut(),
        &mut count,
        &mut handles,
    ))?;
    if handles.is_null() {
        return Err(uefi::LOAD_ERROR);
    }
    let result = (|| {
        if count == 0 || count > 4096 {
            return Err(uefi::LOAD_ERROR);
        }
        let target = pci_resource::selected_identity(cfg!(feature = "xhci-target-msi"));
        let mut found = None;
        for &handle in core::slice::from_raw_parts(handles, count) {
            let mut interface = ptr::null_mut();
            status(services.handle_protocol(handle, &PCI_IO, &mut interface))?;
            if interface.is_null() || interface.addr() & (core::mem::align_of::<PciIo>() - 1) != 0 {
                return Err(uefi::LOAD_ERROR);
            }
            let pci = &mut *interface.cast::<PciIo>();
            let mut config = [0u32; 16];
            pci.read(0, &mut config)?;
            if (config[0] as u16, (config[0] >> 16) as u16) != target || config[2] >> 8 != 0x0c0330
            {
                continue;
            }
            if found.is_some() || config[3] >> 16 & 0x7f != 0 {
                return Err(uefi::LOAD_ERROR);
            }
            let raw_bar = match (config[4] >> 1) & 3 {
                0 => config[4] as u64,
                2 => config[4] as u64 | ((config[5] as u64) << 32),
                _ => return Err(uefi::LOAD_ERROR),
            };
            let (base, length) = pci.resource(services, raw_bar)?;
            let (segment, bus, slot, function) = pci.location()?;
            let record = ControllerResource {
                version: 1,
                flags: 0,
                base,
                length,
                raw_bar,
                segment,
                vendor: target.0,
                device: target.1,
                command_snapshot: config[1] as u16,
                bus,
                slot,
                function,
                bar_index: 0,
                class_revision: config[2],
                reserved: [0; 2],
            };
            record.validate().map_err(|_| uefi::LOAD_ERROR)?;
            found = Some(record);
        }
        found.ok_or(uefi::NOT_FOUND)
    })();
    status(services.free_pool(handles.cast()))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[repr(C)]
    struct MockPci {
        protocol: PciIo,
        config: [u32; 16],
    }
    struct State {
        devices: [MockPci; 2],
        handles: [Handle; 2],
        descriptor: [u8; 48],
        count: usize,
        frees: usize,
    }
    impl State {
        fn new() -> Self {
            let device = || {
                let mut functions = [0; 18];
                functions[6] = read as *const () as usize;
                functions[14] = location as *const () as usize;
                functions[16] = bar as *const () as usize;
                let mut config = [0; 16];
                let (vendor, device) =
                    pci_resource::selected_identity(cfg!(feature = "xhci-target-msi"));
                config[0] = (vendor as u32) | ((device as u32) << 16);
                config[2] = 0x0c033000;
                config[4] = 0xf0000004;
                MockPci {
                    protocol: PciIo {
                        functions,
                        rom_size: 0,
                        rom_image: ptr::null_mut(),
                    },
                    config,
                }
            };
            let mut descriptor = [0; 48];
            descriptor[0] = 0x8a;
            descriptor[1] = 43;
            descriptor[6..14].copy_from_slice(&64u64.to_le_bytes());
            descriptor[14..22].copy_from_slice(&0xf0000000u64.to_le_bytes());
            descriptor[22..30].copy_from_slice(&0x3fffu64.to_le_bytes());
            descriptor[38..46].copy_from_slice(&0x4000u64.to_le_bytes());
            descriptor[46] = 0x79;
            Self {
                devices: [device(), device()],
                handles: [ptr::null_mut(); 2],
                descriptor,
                count: 1,
                frees: 0,
            }
        }
    }
    thread_local! { static STATE:RefCell<State>=RefCell::new(State::new()); }
    extern "efiapi" fn read(
        this: *mut PciIo,
        width: u32,
        offset: u32,
        count: usize,
        out: *mut c_void,
    ) -> Status {
        assert_eq!((width, offset, count), (2, 0, 16));
        unsafe {
            ptr::copy_nonoverlapping((*(this.cast::<MockPci>())).config.as_ptr(), out.cast(), 16)
        };
        0
    }
    extern "efiapi" fn location(
        _: *mut PciIo,
        s: *mut usize,
        b: *mut usize,
        d: *mut usize,
        f: *mut usize,
    ) -> Status {
        unsafe {
            *s = 0;
            *b = 1;
            *d = 0;
            *f = 0
        };
        0
    }
    extern "efiapi" fn bar(_: *mut PciIo, index: u8, _: *mut u64, out: *mut *mut c_void) -> Status {
        assert_eq!(index, 0);
        STATE.with(|s| unsafe { *out = s.borrow_mut().descriptor.as_mut_ptr().cast() });
        0
    }
    extern "efiapi" fn locate(
        _: u32,
        _: *const Guid,
        _: *mut c_void,
        count: *mut usize,
        handles: *mut *mut Handle,
    ) -> Status {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            for i in 0..2 {
                s.handles[i] = ptr::addr_of_mut!(s.devices[i]).cast();
            }
            unsafe {
                *count = s.count;
                *handles = s.handles.as_mut_ptr();
            }
        });
        0
    }
    extern "efiapi" fn protocol(handle: Handle, _: *const Guid, out: *mut *mut c_void) -> Status {
        unsafe { *out = handle };
        0
    }
    extern "efiapi" fn free(_: *mut u8) -> Status {
        STATE.with(|s| s.borrow_mut().frees += 1);
        0
    }
    fn run() -> Result<ControllerResource, Status> {
        let mut services: BootServices = unsafe { core::mem::zeroed() };
        services.functions[36] = locate as *const () as usize;
        services.functions[16] = protocol as *const () as usize;
        services.functions[6] = free as *const () as usize;
        unsafe { discover(&mut services) }
    }
    #[test]
    fn capture_matches_unique_controller_and_frees_protocol_buffers() {
        STATE.with(|s| *s.borrow_mut() = State::new());
        let r = run().unwrap();
        assert_eq!((r.base, r.length, r.bus), (0xf0000000, 0x4000, 1));
        STATE.with(|s| assert_eq!(s.borrow().frees, 2));
    }
    #[test]
    fn capture_missing_duplicate_and_malformed_resources_fail() {
        for scenario in 0..3 {
            STATE.with(|s| {
                let mut state = State::new();
                match scenario {
                    0 => state.devices[0].config[0] = 0xffff,
                    1 => state.count = 2,
                    _ => state.descriptor[0] = 0,
                }
                *s.borrow_mut() = state;
            });
            assert!(run().is_err());
            STATE.with(|s| assert_eq!(s.borrow().frees, if scenario == 0 { 1 } else { 2 }));
        }
    }
}
