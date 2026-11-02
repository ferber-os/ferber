//! Exercise the production console client against deterministic syscall replies.
#[path = "../userspace/common/console_client.rs"]
mod client;
#[path = "../shared/console_protocol.rs"]
mod console_protocol;
#[path = "../shared/echo.rs"]
mod echo;
#[path = "../shared/service_roles.rs"]
mod roles;
mod rt {
    use std::cell::RefCell;
    #[derive(Default)]
    pub struct Fake {
        pub send_result: isize,
        pub receive_result: isize,
        pub sent: Vec<Vec<u8>>,
        pub reply: Vec<u8>,
        pub handles: u32,
    }
    thread_local! { static IO: RefCell<Fake> = RefCell::new(Fake::default()); }
    pub fn reset() {
        IO.with(|io| *io.borrow_mut() = Fake::default());
    }
    pub fn with<R>(f: impl FnOnce(&mut Fake) -> R) -> R {
        IO.with(|io| f(&mut io.borrow_mut()))
    }
    pub fn call(n: usize, args: [usize; 6]) -> isize {
        with(|io| match n {
            8 => {
                assert_eq!(&args[3..], &[0, 0, 0]);
                // The production client retains this bounded buffer throughout the call.
                io.sent.push(
                    unsafe { core::slice::from_raw_parts(args[1] as *const u8, args[2]) }.to_vec(),
                );
                io.send_result
            }
            9 => {
                assert_eq!(&args[3..5], &[0, 0]);
                if io.receive_result != 0 {
                    return io.receive_result;
                }
                assert!(io.reply.len() <= args[2]);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        io.reply.as_ptr(),
                        args[1] as *mut u8,
                        io.reply.len(),
                    );
                    (args[5] as *mut u32).write(io.reply.len() as u32);
                    (args[5] as *mut u32).add(1).write(io.handles);
                }
                0
            }
            _ => panic!("unexpected syscall {n}"),
        })
    }
}
use client::{Client, Error};
use console_protocol as p;
fn reply(
    operation: u16,
    request: u32,
    status: i32,
    argument: u32,
    value: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut bytes = [0; p::HEADER];
    p::encode(
        p::Header {
            operation,
            request,
            status,
            argument,
            value,
        },
        &mut bytes,
    );
    [bytes.as_slice(), payload].concat()
}
#[test]
fn client_does_not_accept_partial_writes_or_mismatched_replies() {
    let valid = reply(p::WRITE, 1, 0, 3, 0, &[]);
    let mut cases = vec![
        reply(p::WRITE, 1, 0, 2, 0, &[]),
        reply(p::WRITE, 1, 0, 3, 1, &[]),
        reply(p::WRITE, 1, 1, 3, 0, &[]),
        reply(p::WRITE, 1, 0, 3, 0, &[1]),
        reply(p::READ, 1, 0, 3, 0, b"abc"),
        reply(p::WRITE, 2, 0, 3, 0, &[]),
    ];
    for len in 0..p::HEADER {
        cases.push(valid[..len].to_vec());
    }
    for bad in cases {
        rt::reset();
        let mut c = Client::new(7);
        c.write(b"abc").unwrap();
        rt::with(|io| io.reply = bad);
        assert_eq!(c.receive(), Err(Error::Invalid));
    }
    rt::reset();
    let mut c = Client::new(7);
    c.write(b"abc").unwrap();
    rt::with(|io| io.reply = valid);
    assert_eq!(c.receive().unwrap().length, 0);
    assert!(!c.pending());
}
#[test]
fn read_reply_counts_errors_and_features_are_exact() {
    for bad in [
        reply(p::READ, 1, 0, 0, 0, &[]),
        reply(p::READ, 1, 0, 3, 0, b"abcd"),
        reply(p::READ, 1, 0, 5, 0, b"abcde"),
        reply(p::READ, 1, -75, 0, 0, &[1]),
        reply(p::READ, 1, -75, 1, 0, &[]),
        reply(p::READ, 1, -75, 0, 1, &[]),
    ] {
        rt::reset();
        let mut c = Client::new(0);
        c.read(4).unwrap();
        rt::with(|io| io.reply = bad);
        assert_eq!(c.receive(), Err(Error::Invalid));
    }
    rt::reset();
    let mut c = Client::new(0);
    c.read(4).unwrap();
    rt::with(|io| io.reply = reply(p::READ, 1, -75, 0, 0, &[]));
    assert_eq!(c.receive(), Err(Error::Remote(-75)));
    assert!(!c.pending());
    c.read(4).unwrap();
    rt::with(|io| io.reply = reply(p::READ, 2, 0, 4, 0, b"abcd"));
    assert_eq!(c.receive().unwrap().length, 4);
    assert_eq!(&c.buffer[p::HEADER..p::HEADER + 4], b"abcd");
    c.info().unwrap();
    rt::with(|io| io.reply = reply(p::INFO, 3, 0, 0, 8, &[]));
    assert_eq!(c.receive(), Err(Error::Invalid));
}
#[test]
fn backpressure_and_pending_requests_cannot_duplicate_or_skip_ids() {
    rt::reset();
    let mut c = Client::new(3);
    rt::with(|io| io.send_result = -11);
    assert_eq!(c.write(b"abc"), Err(Error::System(-11)));
    assert!(!c.pending());
    rt::with(|io| io.send_result = 0);
    c.write(b"abc").unwrap();
    assert_eq!(c.info(), Err(Error::Busy));
    rt::with(|io| {
        assert_eq!(io.sent.len(), 2);
        for packet in &io.sent {
            assert_eq!(p::request(packet).unwrap().request, 1);
        }
        io.receive_result = -11;
    });
    assert_eq!(c.receive(), Err(Error::System(-11)));
    assert!(c.pending());
    rt::with(|io| {
        io.receive_result = 0;
        io.reply = reply(p::WRITE, 1, 0, 3, 0, &[]);
    });
    c.receive().unwrap();
    c.info().unwrap();
    rt::with(|io| assert_eq!(p::request(io.sent.last().unwrap()).unwrap().request, 2));
}
#[test]
fn invalid_requests_never_send_and_unexpected_handles_never_succeed() {
    rt::reset();
    let mut c = Client::new(1);
    assert_eq!(c.read(0), Err(Error::Invalid));
    assert_eq!(c.read(p::MAX_INPUT + 1), Err(Error::Invalid));
    assert_eq!(c.write(&[]), Err(Error::Invalid));
    assert_eq!(c.write(&[0; p::MAX_TEXT + 1]), Err(Error::Invalid));
    rt::with(|io| assert!(io.sent.is_empty()));
    c.info().unwrap();
    rt::with(|io| {
        io.reply = reply(p::INFO, 1, 0, 0, 3, &[]);
        io.handles = 1;
    });
    assert_eq!(c.receive(), Err(Error::Invalid));
}
