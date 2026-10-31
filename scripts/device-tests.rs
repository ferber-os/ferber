//! Exercise production device ownership with deterministic platform failure injection.
#![allow(dead_code)]
extern crate alloc;
use alloc::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[path = "../ferber/src/kernel/device/mod.rs"]
mod device;
#[path = "../shared/device_abi.rs"]
mod wire;
mod kernel {
    pub mod capabilities {
        #[derive(Clone, Copy)]
        pub struct Rights(u32);
        impl Rights {
            pub const READ: Self = Self(1);
            pub const WRITE: Self = Self(2);
            pub const MAP: Self = Self(4);
            pub fn contains(self, r: Self) -> bool {
                self.0 & r.0 == r.0
            }
        }
        impl core::ops::BitOr for Rights {
            type Output = Self;
            fn bitor(self, r: Self) -> Self {
                Self(self.0 | r.0)
            }
        }
        impl core::ops::BitOrAssign for Rights {
            fn bitor_assign(&mut self, r: Self) {
                self.0 |= r.0;
            }
        }
    }
    pub mod memory {
        pub mod paging {
            pub struct MapFlags {
                pub writable: bool,
            }
            impl MapFlags {
                pub fn user_rw() -> Self {
                    Self { writable: true }
                }
                pub fn user_ro() -> Self {
                    Self { writable: false }
                }
            }
        }
        pub mod vmo {
            use super::paging::MapFlags;
            use alloc::sync::Arc;
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            pub struct Vmo {
                pub permits: AtomicUsize,
                pub sealed: AtomicBool,
                pub drops: Arc<AtomicUsize>,
            }
            pub struct WritableMapping {
                v: Arc<Vmo>,
            }
            impl Drop for WritableMapping {
                fn drop(&mut self) {
                    self.v.permits.fetch_sub(1, Ordering::SeqCst);
                }
            }
            impl Drop for Vmo {
                fn drop(&mut self) {
                    self.drops.fetch_add(1, Ordering::SeqCst);
                }
            }
            impl Vmo {
                pub fn validate_owned_range(&self, o: usize, l: usize) -> Result<(), ()> {
                    if o.checked_add(l).is_some_and(|n| n <= 16 * 1024 * 1024) {
                        Ok(())
                    } else {
                        Err(())
                    }
                }
                pub fn mapping_permit(
                    self: &Arc<Self>,
                    f: MapFlags,
                ) -> Result<Option<WritableMapping>, ()> {
                    if f.writable {
                        if self.sealed.load(Ordering::SeqCst) {
                            return Err(());
                        }
                        self.permits.fetch_add(1, Ordering::SeqCst);
                        Ok(Some(WritableMapping {
                            v: Arc::clone(self),
                        }))
                    } else {
                        Ok(None)
                    }
                }
            }
        }
    }
}
use device::{Backend, Error, State};
use kernel::{capabilities::Rights, memory::vmo::Vmo};
#[derive(Clone)]
struct Fake {
    fail: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}
impl Fake {
    fn new() -> Self {
        Self {
            fail: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn step(&self, n: usize) -> Result<(), Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) == n {
            Err(Error::Hardware)
        } else {
            Ok(())
        }
    }
}
// SAFETY: this host backend accesses no hardware. Failure points model uncertain platform operations.
unsafe impl Backend for Fake {
    fn isolate(&mut self) -> Result<(), Error> {
        self.step(1)
    }
    fn install(&mut self, _: usize, _: &Vmo, _: usize, _: usize, _: u64) -> Result<(), Error> {
        self.step(2)
    }
    fn remove(&mut self, _: usize, _: usize) -> Result<(), Error> {
        self.step(3)
    }
    fn start(&mut self) -> Result<(), Error> {
        self.step(4)
    }
    fn quiesce(&mut self) -> Result<(), Error> {
        self.step(5)
    }
}
fn vmo() -> Arc<Vmo> {
    Arc::new(Vmo {
        permits: AtomicUsize::new(0),
        sealed: AtomicBool::new(false),
        drops: Arc::new(AtomicUsize::new(0)),
    })
}
fn request() -> device::abi::DmaRequest {
    device::abi::DmaRequest {
        offset: 0,
        length: 4096,
        permissions: 3,
    }
}
fn rights() -> Rights {
    Rights::MAP | Rights::READ | Rights::WRITE
}
#[test]
fn wire_is_exact_and_rejects_untrusted_addresses() {
    let r = wire::DmaRequest {
        offset: 0,
        length: 4096,
        permissions: 3,
    };
    let b = r.encode().unwrap();
    assert_eq!(wire::DmaRequest::decode(&b).unwrap(), r);
    for n in 0..32 {
        assert!(wire::DmaRequest::decode(&b[..n]).is_err());
    }
    for n in [16, 17, 23, 24, 31] {
        let mut bad = b;
        bad[n] ^= 0x80;
        assert!(wire::DmaRequest::decode(&bad).is_err());
    }
}
#[test]
fn rights_pool_and_running_unmap() {
    let mut d = device::Device::new(Fake::new()).unwrap();
    let v = vmo();
    assert_eq!(
        d.map_dma(v.clone(), Rights::READ, request()),
        Err(Error::Permission)
    );
    let m = d.map_dma(v.clone(), rights(), request()).unwrap();
    assert_eq!(v.permits.load(Ordering::SeqCst), 1);
    d.start().unwrap();
    assert_eq!(d.unmap_dma(m.id), Err(Error::Busy));
    d.stop().unwrap();
    d.unmap_dma(m.id).unwrap();
    assert_eq!(v.permits.load(Ordering::SeqCst), 0);
    assert_eq!(d.unmap_dma(m.id), Err(Error::Invalid));
    d.revoke().unwrap();
    assert_eq!(d.start(), Err(Error::Revoked));
}
#[test]
fn failed_install_and_failed_drain_keep_backing() {
    for failure in [2, 5] {
        let b = Fake::new();
        let mut d = device::Device::new(b.clone()).unwrap();
        let v = vmo();
        let drops = v.drops.clone();
        b.fail.store(failure, Ordering::SeqCst);
        let mapped = d.map_dma(v.clone(), rights(), request());
        if failure == 2 {
            assert_eq!(mapped, Err(Error::Hardware));
            assert_eq!(d.state(), State::Quarantined);
        } else {
            mapped.unwrap();
            assert_eq!(d.stop(), Err(Error::Hardware));
        }
        drop(v);
        drop(d);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn frame_and_mapping_limits_are_independent() {
    let mut d = device::Device::new(Fake::new()).unwrap();
    let v = vmo();
    let mut r = request();
    r.length = 4 * 1024 * 1024;
    d.map_dma(v.clone(), rights(), r).unwrap();
    assert_eq!(d.map_dma(v.clone(), rights(), request()), Err(Error::Quota));
    let mut e = device::Device::new(Fake::new()).unwrap();
    for _ in 0..64 {
        e.map_dma(v.clone(), rights(), request()).unwrap();
    }
    assert_eq!(e.map_dma(v, rights(), request()), Err(Error::Quota));
}
#[test]
fn failed_remove_quarantines_without_releasing_permit() {
    let b = Fake::new();
    let mut d = device::Device::new(b.clone()).unwrap();
    let v = vmo();
    let m = d.map_dma(v.clone(), rights(), request()).unwrap();
    b.fail.store(3, Ordering::SeqCst);
    assert_eq!(d.unmap_dma(m.id), Err(Error::Hardware));
    assert_eq!(v.permits.load(Ordering::SeqCst), 1);
    assert_eq!(d.start(), Err(Error::Quarantined));
}
#[test]
fn unavailable_hardware_never_claims_isolation() {
    assert!(matches!(
        device::Device::new(device::UnavailableBackend),
        Err(Error::Unsupported)
    ));
}
#[test]
fn stale_irq_publications_do_not_cross_generations() {
    let t = device::irq::Token::allocate().unwrap();
    assert!(t.publish());
    assert_eq!(t.take(), Ok(1));
    t.release().unwrap();
    let n = device::irq::Token::allocate().unwrap();
    assert!(!t.publish());
    assert_eq!(t.take(), Err(Error::Revoked));
    assert_eq!(n.pending(), Ok(0));
    for _ in 0..70000 {
        assert!(n.publish());
    }
    assert_eq!(n.take(), Ok(u16::MAX));
    n.release().unwrap();
}

struct TrackedBackend {
    inner: Fake,
    drops: Arc<AtomicUsize>,
}
impl Drop for TrackedBackend {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
// SAFETY: forwards to a host-only backend; Drop records table-ownership release.
unsafe impl Backend for TrackedBackend {
    fn isolate(&mut self) -> Result<(), Error> {
        self.inner.isolate()
    }
    fn install(&mut self, i: usize, v: &Vmo, o: usize, l: usize, p: u64) -> Result<(), Error> {
        self.inner.install(i, v, o, l, p)
    }
    fn remove(&mut self, i: usize, l: usize) -> Result<(), Error> {
        self.inner.remove(i, l)
    }
    fn start(&mut self) -> Result<(), Error> {
        self.inner.start()
    }
    fn quiesce(&mut self) -> Result<(), Error> {
        self.inner.quiesce()
    }
}
#[test]
fn uncertain_hardware_retains_translation_backend_too() {
    for failure in [0, 1, 5] {
        let inner = Fake::new();
        inner.fail.store(failure, Ordering::SeqCst);
        let drops = Arc::new(AtomicUsize::new(0));
        let result = device::Device::new(TrackedBackend {
            inner,
            drops: drops.clone(),
        });
        drop(result);
        assert_eq!(drops.load(Ordering::SeqCst), usize::from(failure == 0));
    }
}
