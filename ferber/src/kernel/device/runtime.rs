//! Weak owner registry permits synchronous device revocation without retaining processes.
#![allow(dead_code)]
use crate::kernel::{sync::SpinLock, task::Process};
use alloc::sync::{Arc, Weak};
use crate::kernel::device::Error;
static OWNERS: SpinLock<[Option<Weak<SpinLock<Process>>>; 64]> =
    SpinLock::new([const { None }; 64]);
pub fn register(process: &Arc<SpinLock<Process>>) -> Result<(), Error> {
    let weak = Arc::downgrade(process);
    let mut owners = OWNERS.lock();
    if owners.iter().flatten().any(|owner| owner.ptr_eq(&weak)) { return Ok(()); }
    let slot = owners.iter().position(|owner| owner.as_ref().is_none_or(|owner| owner.strong_count() == 0))
        .ok_or(Error::Quota)?;
    owners[slot] = Some(weak);
    Ok(())
}
pub fn reconcile() {
    for index in 0..64 {
        let process = {
            let owners = OWNERS.lock();
            owners[index].as_ref().and_then(Weak::upgrade)
        };
        if let Some(process) = process { process.lock().retire_devices(true); }
    }
}
