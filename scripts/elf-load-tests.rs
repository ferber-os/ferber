//! Exercise the production loader with fallible host resources, not real page tables.
#![allow(dead_code)]
extern crate alloc;

use kernel::{memory, scheduler, syscall, task};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

#[path = "../ferber/src/kernel/elf.rs"]
mod elf;
#[path = "../ferber/src/kernel/elf_load.rs"]
mod elf_load;
#[path = "../ferber/src/kernel/sync.rs"]
mod sync;

#[derive(Default)]
struct Resources {
    fail_after: Option<usize>,
    steps: Vec<&'static str>,
    next_id: usize,
    backing: BTreeMap<usize, Vec<u8>>,
    mappings: Vec<(usize, usize, kernel::memory::paging::MapFlags)>,
    tables: usize,
    premature_reclaims: usize,
    tasks: Vec<(
        Arc<sync::SpinLock<kernel::task::Process>>,
        Arc<sync::SpinLock<kernel::task::Thread>>,
    )>,
}
thread_local! { static RESOURCES: RefCell<Resources> = RefCell::new(Resources::default()); }

pub mod capabilities {
    pub struct CapNode;
}

fn fallible(label: &'static str) -> Result<(), ()> {
    RESOURCES.with_borrow_mut(|r| {
        r.steps.push(label);
        match r.fail_after {
            Some(0) => Err(()),
            Some(n) => {
                r.fail_after = Some(n - 1);
                Ok(())
            }
            None => Ok(()),
        }
    })
}

mod kernel {
    pub use crate::capabilities;
    pub(crate) use crate::sync;
    pub mod memory {
        use crate::{fallible, RESOURCES};
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum VmoError {
            OutOfMemory,
            OutOfBounds,
        }
        pub struct VmoFrames {
            pub id: usize,
            pub count: usize,
        }
        pub struct Vmo {
            frames: VmoFrames,
        }
        impl Vmo {
            pub fn create(size: usize) -> Result<Self, VmoError> {
                fallible("VMO allocation").map_err(|_| VmoError::OutOfMemory)?;
                Ok(RESOURCES.with_borrow_mut(|r| {
                    r.next_id += 1;
                    let id = r.next_id;
                    r.backing.insert(id, vec![0; size]);
                    Self {
                        frames: VmoFrames {
                            id,
                            count: size / 4096,
                        },
                    }
                }))
            }
            pub fn frames(&self) -> &VmoFrames {
                &self.frames
            }
            pub fn size(&self) -> usize {
                self.frames.count * 4096
            }
            pub fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), VmoError> {
                fallible("VMO write").map_err(|_| VmoError::OutOfBounds)?;
                RESOURCES.with_borrow_mut(|r| {
                    r.backing.get_mut(&self.frames.id).unwrap()[offset..offset + bytes.len()]
                        .copy_from_slice(bytes);
                });
                Ok(())
            }
        }
        impl Drop for Vmo {
            fn drop(&mut self) {
                RESOURCES.with_borrow_mut(|r| {
                    if r.mappings.iter().any(|(_, id, _)| *id == self.frames.id) {
                        r.premature_reclaims += 1;
                    }
                    assert!(r.backing.remove(&self.frames.id).is_some());
                });
            }
        }
        pub mod paging {
            use super::VmoFrames;
            use crate::{fallible, RESOURCES};
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct MapFlags {
                pub readable: bool,
                pub writable: bool,
                pub executable: bool,
                pub user: bool,
            }
            impl MapFlags {
                pub fn user_rw() -> Self {
                    Self {
                        readable: true,
                        writable: true,
                        executable: false,
                        user: true,
                    }
                }
            }
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub enum PageError {
                OutOfMemory,
            }
            pub struct Tables {
                allocated: usize,
            }
            impl Tables {
                pub fn new() -> Result<Self, PageError> {
                    fallible("page-table root").map_err(|_| PageError::OutOfMemory)?;
                    RESOURCES.with_borrow_mut(|r| r.tables += 1);
                    Ok(Self { allocated: 1 })
                }
            }
            pub trait AddressSpace {
                unsafe fn map_vmo(
                    &mut self,
                    address: usize,
                    frames: &VmoFrames,
                    flags: MapFlags,
                ) -> Result<(), PageError>;
            }
            impl AddressSpace for Tables {
                unsafe fn map_vmo(
                    &mut self,
                    address: usize,
                    frames: &VmoFrames,
                    flags: MapFlags,
                ) -> Result<(), PageError> {
                    let start = RESOURCES.with_borrow(|r| r.mappings.len());
                    for page in 0..frames.count {
                        if fallible("map page").is_err() {
                            // Production AddressSpace::map_vmo rolls back partial leaves.
                            RESOURCES.with_borrow_mut(|r| r.mappings.truncate(start));
                            return Err(PageError::OutOfMemory);
                        }
                        self.allocated += 1;
                        RESOURCES.with_borrow_mut(|r| {
                            r.tables += 1;
                            r.mappings.push((address + page * 4096, frames.id, flags));
                        });
                    }
                    Ok(())
                }
            }
            impl Drop for Tables {
                fn drop(&mut self) {
                    RESOURCES.with_borrow_mut(|r| {
                        r.mappings.clear();
                        r.tables -= self.allocated;
                    });
                }
            }
        }
    }
    pub mod syscall {
        pub const USER_STACK: usize = 0x0000_0020_0000_0000;
        pub const STACK_SIZE: usize = 16384;
    }
    pub mod task {
        use super::{
            memory::{paging::Tables, Vmo},
            sync::SpinLock,
        };
        use crate::fallible;
        use std::sync::Arc;
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum TaskError {
            OutOfMemory,
        }
        #[derive(Default)]
        pub struct Job {
            pub processes: Vec<Arc<SpinLock<Process>>>,
        }
        impl Job {
            pub fn create_process(
                &mut self,
                _: &'static str,
            ) -> Result<Arc<SpinLock<Process>>, TaskError> {
                fallible("job process capacity").map_err(|_| TaskError::OutOfMemory)?;
                let tables = Tables::new().map_err(|_| TaskError::OutOfMemory)?;
                fallible("process ownership allocation").map_err(|_| TaskError::OutOfMemory)?;
                let process = Arc::new(SpinLock::new(Process {
                    tables,
                    backing: Vec::new(),
                    threads: Vec::new(),
                    handles: SpinLock::new(Handles::default()),
                }));
                self.processes.push(process.clone());
                Ok(process)
            }
            pub fn detach_process(&mut self, id: u64) -> Option<Arc<SpinLock<Process>>> {
                assert_eq!(id, 1);
                self.processes.pop()
            }
        }
        #[derive(Default)]
        pub struct Handles {
            entries: Vec<Option<Arc<crate::kernel::capabilities::CapNode>>>,
            count: usize,
        }
        impl Handles {
            pub fn count(&self) -> usize {
                self.count
            }
            pub fn insert_at(
                &mut self,
                slot: u32,
                node: Arc<crate::kernel::capabilities::CapNode>,
            ) {
                let slot = slot as usize;
                if self.entries.len() <= slot {
                    self.entries.resize_with(slot + 1, || None);
                }
                assert!(self.entries[slot].is_none());
                self.entries[slot] = Some(node);
                self.count += 1;
            }
            pub fn slot(&self, slot: usize) -> Option<&Arc<crate::kernel::capabilities::CapNode>> {
                self.entries.get(slot).and_then(Option::as_ref)
            }
        }
        // Match production field order: page-table teardown precedes backing reclaim.
        pub struct Process {
            tables: Tables,
            pub backing: Vec<Vmo>,
            pub threads: Vec<Arc<SpinLock<Thread>>>,
            handles: SpinLock<Handles>,
        }
        impl Process {
            pub fn id(&self) -> u64 {
                1
            }
            pub fn kill(&mut self) {}
            pub fn reserve_backing(&mut self, n: usize) -> Result<(), TaskError> {
                fallible("backing ownership capacity").map_err(|_| TaskError::OutOfMemory)?;
                self.backing.reserve(n);
                Ok(())
            }
            pub fn retain_backing(&mut self, vmo: Vmo) {
                self.backing.push(vmo);
            }
            pub fn address_space_mut(&mut self) -> &mut Tables {
                &mut self.tables
            }
            pub fn handle_table(&self) -> &SpinLock<Handles> {
                &self.handles
            }
            pub fn create_named_thread(
                &mut self,
                _: &'static str,
                entry: usize,
                size: usize,
            ) -> Result<Arc<SpinLock<Thread>>, TaskError> {
                fallible("thread ownership capacity").map_err(|_| TaskError::OutOfMemory)?;
                let kernel_stack = Vmo::create(size).map_err(|_| TaskError::OutOfMemory)?;
                fallible("thread ownership allocation").map_err(|_| TaskError::OutOfMemory)?;
                let thread = Arc::new(SpinLock::new(Thread {
                    entry,
                    kernel_stack,
                    user_stack: None,
                    stack_base: None,
                    initial_sp: None,
                }));
                self.threads.push(thread.clone());
                Ok(thread)
            }
        }
        pub struct Thread {
            pub entry: usize,
            kernel_stack: Vmo,
            pub user_stack: Option<Vmo>,
            pub stack_base: Option<usize>,
            pub initial_sp: Option<usize>,
        }
        impl Thread {
            pub fn set_user_stack(&mut self, stack: Vmo) {
                self.user_stack = Some(stack);
            }
            pub fn set_user_initial_sp(&mut self, sp: usize) -> Result<(), ()> {
                self.initial_sp = Some(sp);
                Ok(())
            }
            pub fn set_user_stack_mapping(&mut self, base: usize) {
                self.stack_base = Some(base);
            }
        }
    }
    pub mod scheduler {
        use super::{
            sync::SpinLock,
            task::{Process, TaskError, Thread},
        };
        use crate::{fallible, RESOURCES};
        use std::sync::Arc;
        pub fn admit(
            process: Arc<SpinLock<Process>>,
            thread: Arc<SpinLock<Thread>>,
        ) -> Result<(), TaskError> {
            for label in [
                "wait token",
                "scheduler task",
                "task registry capacity",
                "ready queue capacity",
            ] {
                fallible(label).map_err(|_| TaskError::OutOfMemory)?;
            }
            RESOURCES.with_borrow_mut(|r| r.tasks.push((process, thread)));
            Ok(())
        }
    }
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x240];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x400120u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&3u16.to_le_bytes());
    for (n, (flags, offset, address, filesz, memsz)) in [
        (5u32, 0x120u64, 0x400120u64, 64u64, 96u64),
        (4, 0x1a0, 0x5001a0, 3, 64),
        (6, 0x230, 0x600230, 5, 0x1800),
    ]
    .into_iter()
    .enumerate()
    {
        let ph = &mut bytes[64 + n * 56..64 + (n + 1) * 56];
        ph[..4].copy_from_slice(&1u32.to_le_bytes());
        ph[4..8].copy_from_slice(&flags.to_le_bytes());
        for (offset_in_ph, value) in [
            (8, offset),
            (16, address),
            (32, filesz),
            (40, memsz),
            (48, 4096),
        ] {
            ph[offset_in_ph..offset_in_ph + 8].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes[0x120..0x160].fill(0xa5);
    bytes[0x1a0..0x1a3].copy_from_slice(b"RO!");
    bytes[0x230..0x235].copy_from_slice(b"hello");
    bytes
}

fn root() -> Arc<sync::SpinLock<kernel::task::Job>> {
    RESOURCES.with_borrow_mut(|r| *r = Resources::default());
    Arc::new(sync::SpinLock::new(kernel::task::Job::default()))
}
fn assert_clean(root: &Arc<sync::SpinLock<kernel::task::Job>>) {
    assert!(root.lock().processes.is_empty());
    RESOURCES.with_borrow(|r| {
        assert!(r.tasks.is_empty(), "scheduler publication on error");
        assert!(r.backing.is_empty(), "VMO leaked after {:?}", r.steps);
        assert!(r.mappings.is_empty(), "mapping leaked after {:?}", r.steps);
        assert_eq!(r.tables, 0, "table resources leaked after {:?}", r.steps);
        assert_eq!(
            r.premature_reclaims, 0,
            "backing reclaimed before unmap after {:?}",
            r.steps
        );
    });
}
fn reap(root: &Arc<sync::SpinLock<kernel::task::Job>>) {
    let tasks = RESOURCES.with_borrow_mut(|r| std::mem::take(&mut r.tasks));
    drop(tasks);
    drop(root.lock().detach_process(1).unwrap());
    assert_clean(root);
}

#[test]
fn every_setup_failure_unwinds_without_publication_or_dangling_backing() {
    let root = root();
    let bytes = fixture();
    assert_eq!(elf_load::admit(&root, &bytes, None), Ok(()));
    let steps = RESOURCES.with_borrow(|r| r.steps.len());
    reap(&root);
    for fail_after in 0..steps {
        RESOURCES.with_borrow_mut(|r| {
            r.steps.clear();
            r.fail_after = Some(fail_after);
        });
        assert!(
            elf_load::admit(&root, &bytes, None).is_err(),
            "failure point {fail_after} did not fail"
        );
        RESOURCES.with_borrow_mut(|r| r.fail_after = None);
        assert_clean(&root);
    }
    assert!(steps >= 20, "fixture omitted multi-page/admission failures");
    assert_eq!(elf_load::admit(&root, &bytes, None), Ok(()));
    reap(&root);
}

#[test]
fn successful_load_copies_only_file_bytes_and_preserves_permissions() {
    let root = root();
    let bytes = fixture();
    elf_load::admit(&root, &bytes, None).unwrap();
    let process = root.lock().processes[0].clone();
    {
        let process = process.lock();
        assert_eq!(process.handle_table().lock().count(), 0);
        assert_eq!(process.backing.len(), 3);
        let thread = process.threads[0].lock();
        assert_eq!(thread.entry, 0x400120);
        assert_eq!(thread.stack_base, Some(kernel::syscall::USER_STACK));
        RESOURCES.with_borrow(|r| {
            for (n, backing) in process.backing.iter().enumerate() {
                let actual = &r.backing[&backing.frames().id];
                let mut expected = vec![0; backing.size()];
                let (offset, source) = match n {
                    0 => (0x120, &bytes[0x120..0x160]),
                    1 => (0x1a0, &bytes[0x1a0..0x1a3]),
                    _ => (0x230, &bytes[0x230..0x235]),
                };
                expected[offset..offset + source.len()].copy_from_slice(source);
                assert_eq!(*actual, expected, "non-file bytes must stay zero");
            }
            let stack = &r.backing[&thread.user_stack.as_ref().unwrap().frames().id];
            let offset = syscall::STACK_SIZE - elf_load::startup::SIZE;
            assert!(stack[..offset].iter().all(|&b| b == 0));
            assert_eq!(
                elf_load::startup::Startup::decode(&stack[offset..]),
                Ok(elf_load::startup::Startup { bootstrap: None })
            );
            assert_eq!(thread.initial_sp, Some(syscall::USER_STACK + offset));
            assert_eq!(thread.initial_sp.unwrap() & 15, 0);
            assert_eq!(r.mappings.len(), 8);
            for &(va, _, flags) in &r.mappings {
                assert!(flags.readable && flags.user);
                assert_eq!(flags.executable, va == 0x400000);
                assert_eq!(flags.writable, va != 0x400000 && va != 0x500000);
            }
            assert_eq!(r.tasks.len(), 1);
        });
    }
    drop(process);
    reap(&root);
}

#[test]
fn explicit_bootstrap_handle_is_discovered_and_all_failures_unwind() {
    let test_root = root();
    let bytes = fixture();
    let node = Arc::new(kernel::capabilities::CapNode);
    let weak = Arc::downgrade(&node);
    elf_load::admit(&test_root, &bytes, Some(node.clone())).unwrap();
    let process = test_root.lock().processes[0].clone();
    {
        let process = process.lock();
        let handles = process.handle_table().lock();
        assert_eq!(handles.count(), 1);
        assert!(Arc::ptr_eq(handles.slot(7).unwrap(), &node));
        assert!(handles.slot(0).is_none());
        let thread = process.threads[0].lock();
        RESOURCES.with_borrow(|r| {
            let stack = &r.backing[&thread.user_stack.as_ref().unwrap().frames().id];
            assert_eq!(
                elf_load::startup::Startup::decode(&stack[syscall::STACK_SIZE - 32..]),
                Ok(elf_load::startup::Startup { bootstrap: Some(7) })
            );
        });
    }
    let steps = RESOURCES.with_borrow(|r| r.steps.len());
    drop(node);
    drop(process);
    reap(&test_root);
    assert!(
        weak.upgrade().is_none(),
        "delegated handle survived process exit"
    );

    for fail_after in 0..steps {
        let failing_root = root();
        let node = Arc::new(kernel::capabilities::CapNode);
        let weak = Arc::downgrade(&node);
        RESOURCES.with_borrow_mut(|r| r.fail_after = Some(fail_after));
        assert!(elf_load::admit(&failing_root, &bytes, Some(node.clone())).is_err());
        drop(node);
        assert!(
            weak.upgrade().is_none(),
            "failure {fail_after} retained delegated handle"
        );
        RESOURCES.with_borrow_mut(|r| r.fail_after = None);
        assert_clean(&failing_root);
    }
}

#[test]
fn invalid_images_and_stack_guard_collisions_allocate_nothing() {
    for address in [
        kernel::syscall::USER_STACK - 4096,
        kernel::syscall::USER_STACK,
        kernel::syscall::USER_STACK + kernel::syscall::STACK_SIZE - 4096,
    ] {
        let root = root();
        let mut bytes = fixture();
        let entry = (address + 0x120) as u64;
        bytes[24..32].copy_from_slice(&entry.to_le_bytes());
        bytes[80..88].copy_from_slice(&entry.to_le_bytes());
        assert_eq!(
            elf_load::admit(&root, &bytes, None),
            Err(elf_load::LoadError::StackOverlap)
        );
        assert!(RESOURCES.with_borrow(|r| r.steps.is_empty()));
        assert_clean(&root);
    }
    let root = root();
    assert!(elf_load::admit(&root, b"not an executable", None).is_err());
    assert!(RESOURCES.with_borrow(|r| r.steps.is_empty()));
    assert_clean(&root);
}

#[test]
fn pages_adjacent_to_the_guard_and_stack_remain_loadable() {
    for address in [
        kernel::syscall::USER_STACK - 8192,
        kernel::syscall::USER_STACK + kernel::syscall::STACK_SIZE,
    ] {
        let root = root();
        let mut bytes = fixture();
        let entry = (address + 0x120) as u64;
        bytes[24..32].copy_from_slice(&entry.to_le_bytes());
        bytes[80..88].copy_from_slice(&entry.to_le_bytes());
        assert_eq!(elf_load::admit(&root, &bytes, None), Ok(()));
        reap(&root);
    }
}
