#![allow(dead_code, unused_imports)]
extern crate alloc;
use std::sync::atomic::{AtomicUsize, Ordering};
static DROPS: AtomicUsize = AtomicUsize::new(0);
static CREATES: AtomicUsize = AtomicUsize::new(0);
static FAIL_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
mod kernel {
    pub mod device {
        pub trait Backend {}
        #[derive(Debug)]
        pub enum Error {
            Invalid,
            Busy,
            Hardware,
            Quota,
        }
    }
    pub mod memory {
        pub mod phys_map {
            pub fn phys_to_virt(a: usize) -> Option<usize> {
                Some(a)
            }
        }
        pub mod vmo {
            pub struct Frames(usize);
            impl Frames {
                pub fn get(&self, n: usize) -> Option<usize> {
                    Some(self.0 + n * 4096)
                }
            }
            pub struct Vmo {
                data: Vec<u64>,
                frames: Frames,
            }
            impl Vmo {
                pub fn create_contiguous(size: usize) -> Result<Self, ()> {
                    let n = crate::CREATES.fetch_add(1, crate::Ordering::SeqCst);
                    if n == crate::FAIL_AT.load(crate::Ordering::SeqCst) {
                        return Err(());
                    }
                    let data = vec![0; size / 8];
                    let base = data.as_ptr() as usize;
                    Ok(Self {
                        data,
                        frames: Frames(base),
                    })
                }
                pub fn size(&self) -> usize {
                    self.data.len() * 8
                }
                pub fn frames(&self) -> &Frames {
                    &self.frames
                }
            }
            impl Drop for Vmo {
                fn drop(&mut self) {
                    crate::DROPS.fetch_add(1, crate::Ordering::SeqCst);
                }
            }
        }
    }
}
mod platform {
    #[derive(Clone, Copy)]
    pub struct MappedRegion {
        pub fail: bool,
    }
    impl MappedRegion {
        pub fn length(&self) -> usize {
            0x4000
        }
        pub fn read64(&self, _: usize) -> Result<u64, ()> {
            Ok(0)
        }
        pub unsafe fn write64(&self, _: usize, _: u64) -> Result<(), ()> {
            if self.fail {
                Err(())
            } else {
                Ok(())
            }
        }
    }
    pub mod hpet {
        #[derive(Clone, Copy)]
        pub struct Clock;
        pub struct Deadline;
        impl Clock {
            pub fn deadline(&self, _: u64) -> Result<Deadline, ()> {
                Ok(Deadline)
            }
            pub fn expired(&self, _: &mut Deadline) -> Result<bool, ()> {
                Ok(false)
            }
        }
    }
    #[path = "COHERENT_PRODUCTION_SOURCE"]
    pub mod iommu;
    #[cfg(test)]
    mod tests {
        use super::{
            hpet::Clock,
            iommu::{
                encoding::{Hardware, CONTROL},
                Coherent,
            },
            MappedRegion,
        };
        use crate::{Ordering, CREATES, DROPS, FAIL_AT};
        fn reset() {
            DROPS.store(0, Ordering::SeqCst);
            CREATES.store(0, Ordering::SeqCst);
            FAIL_AT.store(usize::MAX, Ordering::SeqCst);
        }
        #[test]
        fn actual_coherent_drop_contract() {
            reset();
            drop(Coherent::new(MappedRegion { fail: false }, Clock).unwrap());
            assert_eq!(DROPS.load(Ordering::SeqCst), 7);
            reset();
            let mut c = Coherent::new(MappedRegion { fail: true }, Clock).unwrap();
            assert!(c.write_register(CONTROL, 1).is_err());
            drop(c);
            assert_eq!(
                DROPS.load(Ordering::SeqCst),
                0,
                "failed enable write must retain all seven"
            );
            reset();
            let mut c = Coherent::new(MappedRegion { fail: false }, Clock).unwrap();
            c.write_register(CONTROL, 1).unwrap();
            drop(c);
            assert_eq!(
                DROPS.load(Ordering::SeqCst),
                0,
                "successful enable must retain all seven"
            );
            reset();
            FAIL_AT.store(3, Ordering::SeqCst);
            assert!(Coherent::new(MappedRegion { fail: false }, Clock).is_err());
            assert_eq!(
                DROPS.load(Ordering::SeqCst),
                3,
                "failed allocation before enable frees prior buffers"
            );
            reset();
            let mut c = Coherent::new(MappedRegion { fail: true }, Clock).unwrap();
            assert!(c.write_register(8, 1).is_err());
            drop(c);
            assert_eq!(
                DROPS.load(Ordering::SeqCst),
                7,
                "ordinary pre-enable write failure can reclaim buffers"
            );
        }
    }
}
