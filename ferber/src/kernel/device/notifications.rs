//! IRQ top halves publish atomic tokens; the dispatcher owns channel delivery.
#![allow(dead_code)]
use super::{
    irq::{Token, MAX_BINDINGS},
    Error,
};
use crate::kernel::{
    capabilities::CapNode,
    ipc::channel::{Channel, ChannelError, ChannelMessage},
    memory::accounting,
    scheduler::{register_source, SourceKind, SourceLease},
    sync::SpinLock,
    task::{Process, ProcessState},
};
use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

pub const MESSAGE_SIZE: usize = 32;
pub const EVENT_DEVICE: u32 = 1;
pub const EVENT_SATURATED: u32 = 2;
struct Binding {
    token: Token,
    authority: Weak<CapNode>,
    owner: Weak<SpinLock<Process>>,
    channel: Arc<Channel>,
    source: Option<SourceLease>,
    count: u32,
    bits: u32,
    next_sequence: u64,
    awaiting: Option<u64>,
}
static BINDINGS: SpinLock<[Option<Binding>; MAX_BINDINGS]> =
    SpinLock::new([const { None }; MAX_BINDINGS]);

fn live(binding: &Binding) -> bool {
    binding
        .authority
        .upgrade()
        .is_some_and(|node| node.is_valid())
        && binding
            .owner
            .upgrade()
            .is_some_and(|owner| owner.lock().state() == ProcessState::Running)
}
fn binding_mut(
    bindings: &mut [Option<Binding>; MAX_BINDINGS],
    token: Token,
) -> Result<&mut Binding, Error> {
    bindings[token.slot()]
        .as_mut()
        .filter(|binding| binding.token == token)
        .ok_or(Error::Revoked)
}
/// Caller has checked device WAIT and channel WRITE rights and claimed the device owner.
/// Do not hold a process lock while calling notification APIs.
pub fn bind(
    authority: Weak<CapNode>,
    owner: Weak<SpinLock<Process>>,
    channel: Arc<Channel>,
) -> Result<Token, Error> {
    let token = Token::allocate()?;
    let binding = Binding {
        token,
        authority,
        owner,
        channel,
        source: None,
        count: 0,
        bits: 0,
        next_sequence: 1,
        awaiting: None,
    };
    if !live(&binding) {
        let _ = token.release();
        return Err(Error::Revoked);
    }
    let mut bindings = BINDINGS.lock();
    if bindings[token.slot()].is_some() {
        drop(bindings);
        let _ = token.release();
        return Err(Error::Busy);
    }
    bindings[token.slot()] = Some(binding);
    Ok(token)
}
/// # Safety
/// The platform armed this exact generation with interrupts masked on the current
/// CPU. Its ISR masks the source before publishing; failures must undo hardware arming.
/// No source may produce outside its generation or without this owner's authority.
pub unsafe fn activate(token: Token) -> Result<(), Error> {
    let mut bindings = BINDINGS.lock();
    let binding = binding_mut(&mut bindings, token)?;
    if !live(binding) {
        return Err(Error::Revoked);
    }
    if binding.source.is_some()
        || binding.awaiting.is_some()
        || binding.count != 0
        || token.pending()? != 0
    {
        return Err(Error::Busy);
    }
    binding.source =
        Some(register_source(&binding.channel, SourceKind::Hardware).map_err(|_| Error::Quota)?);
    Ok(())
}
/// Stop future hardware delivery before calling this; queued notifications remain observable.
pub fn deactivate(token: Token) -> Result<(), Error> {
    let removed = {
        let mut bindings = BINDINGS.lock();
        binding_mut(&mut bindings, token)?.source.take()
    };
    drop(removed);
    Ok(())
}
/// # Safety
/// This token's hardware route is masked and cannot be rebound until return.
/// Late publications are inert after token release; device backing remains separately owned.
pub unsafe fn release(token: Token) -> Result<(), Error> {
    let removed = {
        let mut bindings = BINDINGS.lock();
        binding_mut(&mut bindings, token)?;
        token.release()?;
        bindings[token.slot()].take()
    };
    // Closing an endpoint can destroy queued kernel objects; never do that under registry lock.
    drop(removed);
    Ok(())
}
/// A sequence is usable once, only after that notification was delivered.
/// `true` permits the caller to arm again; `false` means another captured event awaits delivery.
pub fn acknowledge(token: Token, generation: u64, sequence: u64) -> Result<bool, Error> {
    if generation != token.generation() {
        return Err(Error::Invalid);
    }
    let mut bindings = BINDINGS.lock();
    let binding = binding_mut(&mut bindings, token)?;
    if !live(binding) {
        return Err(Error::Revoked);
    }
    if binding.awaiting != Some(sequence) {
        return Err(Error::Invalid);
    }
    binding.awaiting = None;
    Ok(binding.count == 0 && token.pending()? == 0)
}

fn encode(token: Token, sequence: u64, count: u32, bits: u32) -> [u8; MESSAGE_SIZE] {
    let mut bytes = [0; MESSAGE_SIZE];
    bytes[..8].copy_from_slice(b"FRIRQ001");
    bytes[8..16].copy_from_slice(&token.generation().to_le_bytes());
    bytes[16..24].copy_from_slice(&sequence.to_le_bytes());
    bytes[24..28].copy_from_slice(&count.to_le_bytes());
    bytes[28..32].copy_from_slice(&bits.to_le_bytes());
    bytes
}
/// Dispatcher context only, with interrupts disabled, after owner/revocation cleanup.
/// No token is released here: only the hardware owner can prove its route is masked.
pub fn drain() {
    let mut bindings = BINDINGS.lock();
    for binding in bindings.iter_mut().flatten() {
        if !live(binding) {
            drop(binding.source.take());
            continue;
        }
        let Ok(count) = binding.token.take() else {
            continue;
        };
        if count != 0 {
            // The ISR masks before publication, so this is no longer an armed wake source.
            drop(binding.source.take());
            binding.count = binding.count.saturating_add(count as u32);
            binding.bits |= EVENT_DEVICE;
            if count == u16::MAX || binding.count == u32::MAX {
                binding.bits |= EVENT_SATURATED;
            }
        }
        if binding.count == 0 || binding.awaiting.is_some() {
            continue;
        }
        let Some(next) = binding.next_sequence.checked_add(1) else {
            continue;
        };
        let encoded = encode(
            binding.token,
            binding.next_sequence,
            binding.count,
            binding.bits,
        );
        let Some(owner) = binding.owner.upgrade() else {
            continue;
        };
        let budget = owner.lock().budget().clone();
        drop(owner);
        let result = accounting::with_budget(&budget, || {
            binding.channel.send_with(MESSAGE_SIZE, 0, || {
                let mut bytes = Vec::new();
                accounting::attempt(|| bytes.try_reserve_exact(MESSAGE_SIZE)).map_err(|error| {
                    match error {
                        accounting::AllocError::Quota => ChannelError::JobQuota,
                        accounting::AllocError::OutOfMemory => ChannelError::OutOfMemory,
                    }
                })?;
                bytes.extend_from_slice(&encoded);
                Ok::<_, ChannelError>(ChannelMessage::new(bytes, Vec::new()))
            })
        });
        if result.is_ok() {
            binding.awaiting = Some(binding.next_sequence);
            binding.next_sequence = next;
            binding.count = 0;
            binding.bits = 0;
        }
    }
}
