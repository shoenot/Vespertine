#![no_std]
#![no_main]

extern crate alloc;
mod launcher;
mod meta;
mod policy;

use vespertine_abi::app::hesper::AppIoMode;
use vespertine_abi::{ProcState, UserID};
use vespertine_abi::tag::{
    CAP_LAUNCHER_CONNECT, CAP_LOG_READER, CAP_SERIAL_WRITE,
};
use vrt::syscall::{
    sys_close,
    sys_yield,
};
use vrt::thread as rt_thread;
use vstd::auth::AuthClient;
use vstd::fs::{
    link_object,
    resolve,
};
use vstd::hesper::recv_hesper_request;
use vstd::log::SystemLog;
use vstd::portal::PortalFactory;
use vstd::{NamespaceRights, prelude::*};
use vstd::proc::Waiter;
use vstd::{
    Exec,
    Process,
};

use crate::launcher::handle_request;
use crate::policy::PolicyStore;

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

fn launch_vlog() -> Result<Process, Error> {
    println!("[INFO] [hesper] Launching vlog...");

    Exec::open(&Path::new("/System/Core/vlog"), "vlog".into())?
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .namespace_rights(NamespaceRights::FULL)
        .grant(CAP_SERIAL_WRITE, AccessRights::READ | AccessRights::CREATE)?
        .spawn()
}

fn launch_vreg(log: &SystemLog) -> Result<Process, Error> {
    log.write_string("Launching vreg".into())?;

    Exec::open(&Path::new("/System/Core/vreg"), "vreg".into())?
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .namespace_rights(NamespaceRights::FULL)
        .spawn()
}

fn launch_auth(log: &SystemLog) -> Result<Process, Error> {
    log.write_string("Launching auth".into())?;

    Exec::open(&Path::new("/System/Core/auth"), "auth".into())?
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .namespace_rights(NamespaceRights::FULL)
        .spawn()
}

fn wait_for_service(name: &str, path: &Path<'_>, process: &Process, log: Option<&SystemLog>) -> Result<(), Error> {
    println!("[INFO] Waiting for {name} service...");
    if let Some(sl) = log {
        let _ = sl.write_string(format!("waiting for {name} service"));
    }
    loop {
        match resolve(path, AccessRights::READ) {
            Ok(handle) => {
                sys_close(handle).map_err(Error::from)?;
                println!("[INFO] {name} service online");
                if let Some(sl) = log {
                    sl.write_string(format!("{name} service online"))?;
                }
                return Ok(());
            }
            Err(error) if error.kind == ErrorKind::NotFound => {
                let info = process.info()
                    .map_err(|error| error.context(format_args!("cannot check {name} service status")))?;
                if info.state == ProcState::Terminated {
                    return Err(Error::unknown(format!(
                        "{name} service {} with status {} before publishing '{}'",
                        info.short_status(), info.status_code(), path.as_str()
                    )));
                }
                sys_yield();
            }
            Err(error) => return Err(error.context(format_args!("waiting for {name} service"))),
        }
    }
}

fn connect_auth() -> Result<AuthClient, Error> {
    let context = |error: Error| error.context("cannot connect to Auth");
    for _ in 0..100 {
        match AuthClient::connect() {
            Ok(client) => return Ok(client),
            Err(error) if error.kind == ErrorKind::NotFound => sys_yield(),
            Err(error) => return Err(context(error)),
        }
    }

    AuthClient::connect().map_err(context)
}

#[vapp::main]
fn main(_pkg: &ProcessInitPackage) -> Result<(), Error> {
    let launcher_policy = Arc::new(PolicyStore::load()?);

    let vlog = launch_vlog()?;
    wait_for_service("vlog", &Path::new("/System/Services/Log"), &vlog, None)?;
    let log = SystemLog::connect();

    let auth = launch_auth(&log)?;
    wait_for_service("auth", &Path::new("/System/Services/Auth"), &auth, Some(&log))?;
    drop(auth);

    let vreg = launch_vreg(&log)?;
    wait_for_service("vreg", &Path::new("/System/Services/VRegistry"), &vreg, Some(&log))?;
    drop(vreg);

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

    let terminal = launcher::spawn_local(
        "terminal", 
        AppIoMode::Terminal, 
        &launcher_policy, 
        UserID(default_user.user.id), 
        launcher::LaunchHandles {
            source: env::source(),
            sink: env::sink(),
            cwd: env::cwd(),
        }
    )?;
    terminal.resume()?;

    log.debug("Test message")?;
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
