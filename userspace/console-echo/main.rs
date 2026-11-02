#![no_std]
#![no_main]
#[path = "../common/console_client.rs"]
mod console_client;
#[path = "../../shared/console_protocol.rs"]
mod console_protocol;
#[path = "../../shared/echo.rs"]
mod echo;
#[path = "../../shared/service_roles.rs"]
mod roles;
#[path = "../common/runtime.rs"]
mod rt;
#[path = "../../shared/service_protocol.rs"]
mod service;
#[path = "../../shared/startup.rs"]
mod startup;
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("../common/riscv64.S"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("../common/x86_64.S"));
use console_client::{Client, Error, Response};
fn control(handle: usize, session: usize) {
    let mut bytes = [0; service::SIZE];
    rt::recv(handle, &mut bytes);
    match service::decode(&bytes) {
        Some((service::PING, value)) => rt::send(handle, &service::encode(service::PONG, value)),
        Some((service::SHUTDOWN, 0)) => {
            rt::close(session);
            rt::close(handle);
            rt::exit(0)
        }
        _ => rt::exit(3),
    }
}
fn response(client: &mut Client, bootstrap: usize) -> Result<Response, Error> {
    loop {
        match rt::wait(&[bootstrap as u32, client.handle as u32]) {
            0 => control(bootstrap, client.handle),
            1 => match client.receive() {
                Err(Error::System(-11)) => continue,
                result => return result,
            },
            _ => rt::exit(4),
        }
    }
}
fn write(client: &mut Client, bootstrap: usize, bytes: &[u8]) {
    client.write(bytes).unwrap_or_else(|_| rt::exit(5));
    response(client, bootstrap).unwrap_or_else(|_| rt::exit(5));
}
#[no_mangle]
pub unsafe extern "C" fn user_main(record: *const u8) -> ! {
    if record.is_null() || record.addr() & 15 != 0 {
        rt::exit(1)
    }
    let bootstrap =
        match startup::Startup::decode(core::slice::from_raw_parts(record, startup::SIZE)) {
            Ok(startup::Startup {
                bootstrap: Some(handle),
            }) => handle as usize,
            _ => rt::exit(1),
        };
    let mut bytes = [0; roles::SIZE];
    rt::recv(bootstrap, &mut bytes);
    let grants = roles::Roles::decode(&bytes).unwrap_or_else(|_| rt::exit(2));
    if grants.role != roles::ECHO || grants.handles[0] as usize == bootstrap {
        rt::exit(2)
    }
    let mut client = Client::new(grants.handles[0] as usize);
    rt::send(bootstrap, &service::encode(service::READY, 0));
    // Lifecycle challenges remain serviceable while the console waits for hardware input.
    client.info().unwrap_or_else(|_| rt::exit(5));
    if response(&mut client, bootstrap)
        .unwrap_or_else(|_| rt::exit(5))
        .value
        & 3
        != 3
    {
        rt::exit(5)
    }
    write(&mut client, bootstrap, echo::PROMPT);
    let mut echo = echo::Echo::new();
    let mut input = [0; console_protocol::MAX_INPUT];
    let mut output = [0; echo::MAX_OUTPUT];
    loop {
        client.read(input.len()).unwrap_or_else(|_| rt::exit(5));
        let length = match response(&mut client, bootstrap) {
            Ok(reply) => reply.length,
            Err(Error::Remote(-75)) => {
                echo = echo::Echo::new();
                write(&mut client, bootstrap, b"\r\n[input lost]\r\necho> ");
                continue;
            }
            Err(_) => rt::exit(5),
        };
        input[..length].copy_from_slice(
            &client.buffer[console_protocol::HEADER..console_protocol::HEADER + length],
        );
        for byte in &input[..length] {
            let count = echo.input(*byte, &mut output);
            if count != 0 {
                write(&mut client, bootstrap, &output[..count]);
            }
        }
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    rt::exit(99)
}
