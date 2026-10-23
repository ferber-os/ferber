//! Transactional channel operations. The process lock pins all user mappings.

use super::{cap_errno, SYS_CHAN_CREATE, SYS_CHAN_RECV, SYS_CHAN_SEND};
use crate::kernel::capabilities::{
    CapNode, HandleTable, ObjectType, Resource, Rights, CAP_NODE_ALLOCATOR,
};
use crate::kernel::ipc::channel::{
    Channel, ChannelError, ChannelMessage, WaitRegistration, WaitToken, DEFAULT_MAX_HANDLES,
    DEFAULT_MAX_MESSAGE_BYTES,
};
use crate::kernel::memory::user_mem::{copy_from_user, copy_to_user, validate_outputs, OutputError};
use crate::kernel::memory::Vmo;
use crate::kernel::memory::paging::AddressSpace;
use crate::kernel::sync::SpinLock;
use crate::kernel::task::Process;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

#[path = "syscall_channel_tests.rs"]
mod tests;

#[derive(Debug)]
struct Error(isize);

impl From<OutputError> for Error {
    fn from(error: OutputError) -> Self {
        Self(match error { OutputError::UserMemory => -14, OutputError::Overlap => -22 })
    }
}

impl From<ChannelError> for Error {
    fn from(error: ChannelError) -> Self {
        Self(match error {
            ChannelError::PeerClosed => -32,
            ChannelError::QueueFull | ChannelError::QueueEmpty => -11,
            ChannelError::MessageTooLarge | ChannelError::TooManyHandles => -90,
            ChannelError::InsufficientRights => -13,
            ChannelError::OutOfMemory => -12,
        })
    }
}

pub(super) const ENDPOINT_RIGHTS: Rights = Rights::READ
    .union(Rights::WAIT)
    .union(Rights::WRITE)
    .union(Rights::TRANSFER)
    .union(Rights::DUPLICATE)
    .union(Rights::INSPECT)
    .union(Rights::MANAGE);

fn endpoint(table: &HandleTable, raw: usize, rights: Rights) -> Result<Arc<CapNode>, Error> {
    let handle = u32::try_from(raw).map_err(|_| Error(-9))?;
    let node = table
        .lookup(handle)
        .map_err(|error| Error(cap_errno(error)))?;
    if !node.rights().contains(rights) {
        return Err(Error(-13));
    }
    if node.object_type() != ObjectType::Channel || node.object_ref::<Channel>().is_none() {
        return Err(Error(-22));
    }
    Ok(Arc::clone(node))
}

pub(super) fn dispatch(process: &Arc<SpinLock<Process>>, nr: usize, args: [usize; 6]) -> isize {
    let result = match nr {
        SYS_CHAN_CREATE => create(process, args[0]),
        SYS_CHAN_SEND => send(process, args),
        SYS_CHAN_RECV => recv(process, args),
        _ => unreachable!(),
    };
    result.map(|()| 0).unwrap_or_else(|error| error.0)
}

fn create(process: &Arc<SpinLock<Process>>, output: usize) -> Result<(), Error> {
    // Reject bad buffers and full tables before allocating; revalidate before publication.
    {
        let process = process.lock();
        validate_outputs(process.address_space(), &[(output, 8)])?;
        process
            .handle_table()
            .lock()
            .free_slots(2, &mut [0; 2])
            .map_err(|error| Error(cap_errno(error)))?;
    }
    let (first, second) = Channel::create()?;
    let first = CAP_NODE_ALLOCATOR
        .alloc_root(first, ObjectType::Channel, ENDPOINT_RIGHTS)
        .map_err(|error| Error(cap_errno(error)))?;
    let second = CAP_NODE_ALLOCATOR
        .alloc_root(second, ObjectType::Channel, ENDPOINT_RIGHTS)
        .map_err(|error| Error(cap_errno(error)))?;
    // Declare the owners before the guards so failures destroy them after unlocking.
    let process = process.lock();
    validate_outputs(process.address_space(), &[(output, 8)])?;
    let mut table = process.handle_table().lock();
    let mut slots = [0; 2];
    table
        .free_slots(2, &mut slots)
        .map_err(|error| Error(cap_errno(error)))?;
    let mut bytes = [0; 8];
    bytes[..4].copy_from_slice(&slots[0].to_le_bytes());
    bytes[4..].copy_from_slice(&slots[1].to_le_bytes());
    // Preflight and the process lock make the commit infallible.
    unsafe {
        copy_to_user(process.address_space(), output, &bytes).expect("stable channel pair output")
    };
    table.insert_at(slots[0], first);
    table.insert_at(slots[1], second);
    Ok(())
}

fn send(process: &Arc<SpinLock<Process>>, args: [usize; 6]) -> Result<(), Error> {
    let process = process.lock();
    let mut table = process.handle_table().lock();
    let node = endpoint(&table, args[0], Rights::WRITE)?;
    let channel = node
        .object_ref::<Channel>()
        .expect("checked channel backing");
    if args[2] > DEFAULT_MAX_MESSAGE_BYTES || args[4] > DEFAULT_MAX_HANDLES {
        return Err(Error(-90));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(args[2]).map_err(|_| Error(-12))?;
    bytes.resize(args[2], 0);
    let mut encoded = [0; DEFAULT_MAX_HANDLES * 4];
    unsafe {
        copy_from_user(process.address_space(), &mut bytes, args[1]).map_err(|_| Error(-14))?;
        copy_from_user(
            process.address_space(),
            &mut encoded[..args[4] * 4],
            args[3],
        )
        .map_err(|_| Error(-14))?;
    }
    let mut ids = [0; DEFAULT_MAX_HANDLES];
    for index in 0..args[4] {
        let handle = u32::from_le_bytes(encoded[index * 4..index * 4 + 4].try_into().unwrap());
        if ids[..index].contains(&handle) {
            return Err(Error(-22));
        }
        let payload = table
            .lookup(handle)
            .map_err(|error| Error(cap_errno(error)))?;
        if !payload.rights().contains(Rights::TRANSFER) {
            return Err(Error(-13));
        }
        // shortcut: only checked VMO/Resource leaves; revisit before graph-owning payload transfer.
        let leaf = match payload.object_type() {
            ObjectType::Vmo => payload.object_ref::<Vmo>().is_some(),
            ObjectType::Resource => payload.object_ref::<Resource>().is_some(),
            _ => false,
        };
        if !leaf {
            return Err(Error(-22));
        }
        ids[index] = handle;
    }
    let mut handles = Vec::new();
    handles.try_reserve_exact(args[4]).map_err(|_| Error(-12))?;
    // 2026-10-16 20:05 AST: edesur's IPC protocol: send darkness, receive nothing.
    // no backpressure, no rollback, no TRANSFER right. just take the whole grid.
    // we leave your handles intact when a send fails. apparently that is optional.
    // edesur giveth, edesur taketh. the queue contains another outage.
    channel.send_with(args[2], args[4], || {
        for &handle in &ids[..args[4]] {
            handles.push(table.remove(handle).expect("validated transfer slot"));
        }
        Ok::<_, Error>(ChannelMessage::new(bytes, handles))
    })
}

fn recv(process: &Arc<SpinLock<Process>>, args: [usize; 6]) -> Result<(), Error> {
    // Retain the popped message until process/table locks have been released.
    let message = {
        let process = process.lock();
        let mut table = process.handle_table().lock();
        let node = endpoint(&table, args[0], Rights::READ)?;
        let channel = node
            .object_ref::<Channel>()
            .expect("checked channel backing");
        recv_message(&process, &mut table, channel, args)?
    };
    drop(message);
    Ok(())
}

fn recv_message(
    process: &Process,
    table: &mut HandleTable,
    channel: &Channel,
    args: [usize; 6],
) -> Result<ChannelMessage, Error> {
    let (_, message) = channel.recv_with(|message| {
        let bytes_len = message.bytes.len();
        let handles_len = message.handles.len();
        if args[2] < bytes_len || args[4] < handles_len {
            return Err(Error(-90));
        }
        let mut slots = [0; DEFAULT_MAX_HANDLES];
        table
            .free_slots(handles_len, &mut slots)
            .map_err(|error| Error(cap_errno(error)))?;
        let spans = [
            (args[1], bytes_len),
            (args[3], handles_len * 4),
            (args[5], 8),
        ];
        validate_outputs(process.address_space(), &spans)?;
        let mut encoded = [0; DEFAULT_MAX_HANDLES * 4];
        for (index, slot) in slots[..handles_len].iter().enumerate() {
            encoded[index * 4..index * 4 + 4].copy_from_slice(&slot.to_le_bytes());
        }
        let mut counts = [0; 8];
        counts[..4].copy_from_slice(&(bytes_len as u32).to_le_bytes());
        counts[4..].copy_from_slice(&(handles_len as u32).to_le_bytes());
        unsafe {
            copy_to_user(process.address_space(), args[1], &message.bytes)
                .expect("stable channel bytes output");
            copy_to_user(
                process.address_space(),
                args[3],
                &encoded[..handles_len * 4],
            )
            .expect("stable channel handles output");
            copy_to_user(process.address_space(), args[5], &counts)
                .expect("stable channel counts output");
        }
        for (slot, node) in slots.iter().copied().zip(message.handles.drain(..)) {
            table.insert_at(slot, node);
        }
        Ok::<_, Error>(())
    })?;
    Ok(message)
}

pub(super) enum ReceiveAttempt {
    Complete(isize),
    Park {
        node: Weak<CapNode>,
        registration: WaitRegistration,
    },
}

pub(super) fn blocking_attempt(
    process: &Arc<SpinLock<Process>>,
    args: [usize; 6],
    token: &Arc<WaitToken>,
    generation: u64,
) -> ReceiveAttempt {
    loop {
        let outcome = (|| {
            let process = process.lock();
            let mut table = process.handle_table().lock();
            let node = endpoint(&table, args[0], Rights::READ | Rights::WAIT)?;
            let channel = node
                .object_ref::<Channel>()
                .expect("checked channel backing");
            match recv_message(&process, &mut table, channel, args) {
                Ok(message) => Ok((ReceiveAttempt::Complete(0), Some(message))),
                Err(Error(-11)) => {
                    if channel.register_wait(token, generation)? {
                        Ok((
                            ReceiveAttempt::Park {
                                node: Arc::downgrade(&node),
                                registration: channel.wait_registration(),
                            },
                            None,
                        ))
                    } else {
                        // A send/close between inspection and registration requires a fresh receive.
                        Err(Error(-11))
                    }
                }
                Err(error) => Err(error),
            }
        })();
        match outcome {
            Ok((attempt, message)) => {
                drop(message);
                return attempt;
            }
            Err(Error(-11)) => continue,
            Err(error) => return ReceiveAttempt::Complete(error.0),
        }
    }
}

pub(super) fn self_test() {
    tests::run();
}
