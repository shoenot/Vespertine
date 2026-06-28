#![no_std]
#![no_main]

extern crate alloc;
mod launcher;
mod meta;
mod parse;
mod policy;
use alloc::format;
use alloc::sync::Arc;

use vespertine_abi::tag::{
    CAP_LAUNCHER_CONNECT,
    CAP_LOGGER,
};
use vespertine_abi::{
    AccessRights,
    HandleID,
    ProcessInitPackage, UserID,
};
use vespertine_rt::syscall::{sys_close, sys_yield};
use vespertine_rt::{
    println,
    thread as rt_thread,
};
use vespertine_std::fs::{
    Path,
    link_object,
    resolve,
};
use vespertine_std::hesper::recv_hesper_request;
use vespertine_std::log::SystemLog;
use vespertine_std::portal::PortalFactory;
use vespertine_std::proc::Waiter;
use vespertine_std::socket::Socket;
use vespertine_std::{
    Error, ErrorKind, Exec, Process, Read, Write, env
};

use crate::launcher::handle_request;
use crate::policy::PolicyStore;

use vespertine_std::auth::AuthClient;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] Hesper error: {:?}", e);
    }
    let _ = sys_close(pkg.sink_handle);
}

struct LauncherAccept {
    session: HandleID,
    caller_process: HandleID,
}

fn recv_launcher_accept(socket: &Socket) -> Result<LauncherAccept, Error> {
    let mut bytes = [0u8; 8];
    socket.read_exact(&mut bytes)?;
    Ok(LauncherAccept {
        session: HandleID(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize),
        caller_process: HandleID(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize),
    })
}

fn spawn_launcher_session(handle: HandleID, caller_process: HandleID, policy: Arc<PolicyStore>) -> Result<(), Error> {
    rt_thread::spawn(move || {
        let log = SystemLog::connect();
        let socket = Socket::from_handle(handle);
        let caller = Process::from_handle(caller_process);
        let caller_info = match caller.info() {
            Ok(info) => info,
            Err(e) => {
                let _ = log.write_string(format!("failed to inspect launcher caller: {:?}", e));
                return;
            }
        };

        loop {
            let request = match recv_hesper_request(&socket) {
                Ok(request) => request,
                Err(error) if error.kind == ErrorKind::EndOfStream => {
                    break;
                }
                Err(error) => {
                    let _ = log.write_string(format!("launcher session failed: {:?}", error,));
                    break;
                }
            };

            if let Err(error) = handle_request(&socket, request, &log, &policy, caller_info.user) {
                let _ = log.write_string(format!("Hesper request failed: {:?}", error,));
            }
        }
    })
    .map(|_| ())
    .map_err(Error::from)
}

fn launch_vreg(log: &SystemLog) -> Result<(), Error> {
    log.write_string("Launching vreg".into())?;

    Exec::open(&Path::new("/System/Core/vreg"), "vreg".into())?
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .root_rights(AccessRights::all())
        .grant(CAP_LOGGER, AccessRights::WRITE)?
        .spawn()?;

    Ok(())
}

fn wait_for_vreg(log: &SystemLog) -> Result<(), Error> {
    println!("[INFO] Waiting for vreg service...");
    let _ = log.write_string("waiting for vreg service".into());
    loop {
        match resolve(&Path::new("/System/Services/VRegistry"), AccessRights::READ) {
            Ok(handle) => {
                sys_close(handle).map_err(Error::from)?;
                println!("[INFO] vreg service online");
                log.write_string("vreg service online".into())?;
                return Ok(());
            },
            Err(_) => {
                sys_yield();
            },
        }
    }
}

fn launch_auth(log: &SystemLog) -> Result<(), Error> {
    log.write_string("Launching auth".into())?;

    Exec::open(&Path::new("/System/Core/auth"), "auth".into())?
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .root_rights(AccessRights::all())
        .grant(CAP_LOGGER, AccessRights::WRITE)?
        .spawn()?;

    Ok(())
}

fn wait_for_auth(log: &SystemLog) -> Result<(), Error> {
    println!("[INFO] Waiting for auth service...");
    let _ = log.write_string("waiting for auth service".into());

    loop {
        match resolve(&Path::new("/System/Services/Auth"), AccessRights::READ) {
            Ok(handle) => {
                sys_close(handle).map_err(Error::from)?;
                println!("[INFO] auth service online");
                log.write_string("auth service online".into())?;
                return Ok(());
            },
            Err(_) => {
                sys_yield();
            },
        }
    }
}

fn connect_auth() -> Result<AuthClient, Error> {
    for _ in 0..100 {
        match AuthClient::connect() {
            Ok(client) => return Ok(client),
            Err(_) => sys_yield(),
        }
    }

    AuthClient::connect()
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let launcher_policy = Arc::new(PolicyStore::load()?);
    let log = SystemLog::connect();
    println!("[INFO] Hesper init system online");
    log.write_string("Hesper init system online".into())?;

    launch_auth(&log)?;
    wait_for_auth(&log)?;

    launch_vreg(&log)?;
    wait_for_vreg(&log)?;

    let portal_factory = PortalFactory::request()?;
    let (launcher_portal, launcher_accept) = portal_factory.create(CAP_LAUNCHER_CONNECT, AccessRights::READ | AccessRights::WRITE)?;
    let services = resolve(&Path::new("/System/Services"), AccessRights::CREATE)?;
    link_object(services, "Launcher", launcher_portal)?;

    // namespace owns an arc to the portal so closing it here is safe
    sys_close(launcher_portal).map_err(Error::from)?;
    sys_close(services).map_err(Error::from)?;
    drop(portal_factory);

    let launcher_accept = Socket::from_handle(launcher_accept);

    let mut auth = connect_auth()?;
    let default_user = auth.default_user()?;

    println!("[INFO] Launching terminal...");
    log.write_string("Launching terminal".into())?;

    Exec::open_canonical("terminal")?
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .root_rights(AccessRights::all())
        .user(UserID(default_user.user.id))
        .grant(CAP_LOGGER, AccessRights::WRITE)?
        .spawn()?;

    println!("[INFO] Hesper initialization complete. Entering event loop.");

    let mut waiter = Waiter::new().readable(launcher_accept.handle());

    loop {
        waiter.wait()?;

        if waiter.ready(0) {
            match recv_launcher_accept(&launcher_accept) {
                Ok(accept) => {
                    if let Err(e) = spawn_launcher_session(accept.session, accept.caller_process, launcher_policy.clone()) {
                        let _ = log.write_string(format!("failed to spawn launcher. session: {:?}", e));
                    }
                }
                Err(e) => {
                    let _ = log.write_string(format!("invalid launcher accept. message: {:?}", e));
                }
            }
        }

        waiter.clear();
    }
}
