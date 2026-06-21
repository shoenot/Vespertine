#![no_std]
#![no_main]

extern crate alloc;
mod launcher;
mod meta;
use alloc::format;

use vespertine_abi::tag::{
    CAP_LAUNCHER_CONNECT, CAP_LOGGER
};
use vespertine_abi::{
    AccessRights, HandleID, ProcessInitPackage
};
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_rt::thread as rt_thread;
use vespertine_std::fs::{Path, link_object, resolve};
use vespertine_std::hesper::recv_hesper_request;
use vespertine_std::log::SystemLog;
use vespertine_std::portal::PortalFactory;
use vespertine_std::proc::Waiter;
use vespertine_std::socket::Socket;
use vespertine_std::{
    Error, Exec, Read, Write, env
};

use crate::launcher::handle_request;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] Hesper error: {:?}", e);
    }
    let _ = sys_close(pkg.sink_handle);
}

fn recv_launcher_accept(socket: &Socket) -> Result<HandleID, Error> {
    let mut bytes = [0u8; 4];
    socket.read_exact(&mut bytes)?;
    Ok(HandleID(u32::from_le_bytes(bytes) as usize))
}

fn spawn_launcher_session(handle: HandleID) -> Result<(), Error> {
    rt_thread::spawn(move || {
        let log = SystemLog::connect();
        let socket = Socket::from_handle(handle);

        loop {
            let request = match recv_hesper_request(&socket) {
                Ok(r) => r,
                Err(e) => {
                    let _ = log.write_string(format!("launcher session ended: {:?}", e));
                    break;
                },
            };
            if let Err(e) = handle_request(&socket, request, &log) {
                let _ = log.write_string(format!("Hesper request failed: {:?}", e));
            }
        }
    })
    .map(|_| ())
    .map_err(Error::from)
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let log = SystemLog::connect();
    println!("[INFO] Hesper init system online");
    log.write_string("Hesper init system online".into())?;

    let portal_factory = PortalFactory::request()?;
    let (launcher_portal, launcher_accept) = portal_factory.create(
        CAP_LAUNCHER_CONNECT, 
        AccessRights::READ | AccessRights::WRITE
    )?;
    let services = resolve(
        &Path::new("/System/Services"), 
        AccessRights::CREATE
    )?;
    link_object(services, "Launcher", launcher_portal)?;

    // namespace owns an arc to the portal so closing it here is safe
    sys_close(launcher_portal).map_err(Error::from)?;
    sys_close(services).map_err(Error::from)?;
    drop(portal_factory);

    let launcher_accept = Socket::from_handle(launcher_accept);

    println!("[INFO] Launching terminal...");
    log.write_string("Launching terminal".into())?;

    Exec::new("terminal".into())
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .root_rights(AccessRights::all())
        .grant(CAP_LOGGER, AccessRights::WRITE)?
        .spawn()?;

    println!("[INFO] Hesper initialization complete. Entering event loop.");

    let mut waiter = Waiter::new().readable(launcher_accept.handle());

    loop {
        waiter.wait()?;

        if waiter.ready(0) {
            match recv_launcher_accept(&launcher_accept) {
                Ok(session) => {
                    if let Err(e) = spawn_launcher_session(session) {
                        let _ = log.write_string(format!("failed to spawn launcher. session: {:?}", e));
                    }
                },
                Err(e) => {
                    let _ = log.write_string(format!("invalid launcher accept. message: {:?}", e));
                },
            }
        }

        waiter.clear();
    }
}
