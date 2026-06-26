#![no_std]
#![no_main]

extern crate alloc;

mod registry;

use alloc::format;
use alloc::sync::Arc;

use vespertine_abi::app::vreg::{
    VREG_STATUS_INTERNAL_ERROR,
    VREG_STATUS_OK,
};
use vespertine_abi::tag::CAP_VREG_CONNECT;
use vespertine_abi::{
    AccessRights,
    HandleID,
    ProcessInitPackage,
};
use vespertine_rt::syscall::sys_close;
use vespertine_rt::{
    println,
    thread as rt_thread,
};
use vespertine_std::fs::{
    Path,
    link_object,
    resolve,
};
use vespertine_std::log::SystemLog;
use vespertine_std::portal::PortalFactory;
use vespertine_std::proc::Waiter;
use vespertine_std::socket::Socket;
use vespertine_std::vreg::{
    VRegistryRequest,
    recv_vreg_request,
    send_list_end,
    send_list_entry,
    send_resolve_response,
    status_from_error,
};
use vespertine_std::{
    Error,
    ErrorKind,
    Read,
    Write,
};

use crate::registry::AppRegistry;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };

    if let Err(error) = run() {
        println!("[ERROR] vreg failed: {:?}", error);
    }

    let _ = sys_close(pkg.sink_handle);
}

fn recv_vreg_accept(socket: &Socket) -> Result<HandleID, Error> {
    let mut bytes = [0u8; 4];
    socket.read_exact(&mut bytes)?;
    Ok(HandleID(u32::from_le_bytes(bytes) as usize))
}

fn handle_request(socket: &Socket, request: VRegistryRequest, registry: &AppRegistry, log:
&SystemLog) -> Result<(), Error> {
    match request {
        VRegistryRequest::Resolve { request_id, name } => {
            match registry.resolve(&name) {
                Ok(app) => send_resolve_response(socket, request_id, VREG_STATUS_OK, Some(&app)),
                Err(error) => {
                    let _ = log.write_string(format!("vreg resolve failed for {}: {:?}", name, error));
                    send_resolve_response(socket, request_id, status_from_error(&error), None)?;
                    Err(error)
                }
            }
        },
        VRegistryRequest::List { request_id } => {
            let entries = match registry.list() {
                Ok(entries) => entries,
                Err(error) => {
                    let _ = log.write_string(format!("vreg list failed: {:?}", error));
                    send_list_end(socket, request_id, status_from_error(&error))?;
                    return Err(error);
                }
            };
            for entry in entries {
                send_list_entry(socket, request_id, &entry)?;
            }
            send_list_end(socket, request_id, VREG_STATUS_OK)
        },
    }
}

fn spawn_vreg_session(handle: HandleID, registry: Arc<AppRegistry>) -> Result<(), Error> {
    rt_thread::spawn(move || {
        let log = SystemLog::connect();
        let socket = Socket::from_handle(handle);
        loop {
            let request = match recv_vreg_request(&socket) {
                Ok(request) => request,
                Err(error) if error.kind == ErrorKind::EndOfStream => { break; },
                Err(error) => {
                    let _ = log.write_string(format!("vreg session failed: {:?}", error));
                    break;
                },
            };

            if let Err(error) = handle_request(&socket, request, &registry, &log) {
                if error.kind != ErrorKind::NotFound {
                    let _ = log.write_string(format!("vreg request failed: {:?}", error));
                }
            }
        }
    })
    .map(|_| ())
    .map_err(Error::from)
}

fn publish_service() -> Result<Socket, Error> {
    let portal_factory = PortalFactory::request()?;
    let (portal, accept) = portal_factory.create(CAP_VREG_CONNECT, AccessRights::READ | AccessRights::WRITE)?;

    let services = resolve(&Path::new("/System/Services"), AccessRights::CREATE)?;
    link_object(services, "VRegistry", portal)?;

    sys_close(portal).map_err(Error::from)?;
    sys_close(services).map_err(Error::from)?;

    Ok(Socket::from_handle(accept))
}

fn run() -> Result<(), Error> {
    let log = SystemLog::connect();

    println!("[INFO] vreg starting");
    log.write_string("vreg starting".into())?;

    println!("[INFO] vreg loading registry");
    log.write_string("vreg loading registry".into())?;

    let registry = match AppRegistry::load() {
        Ok(registry) => Arc::new(registry),
        Err(error) => {
            println!("[ERROR] vreg registry load failed: {:?}", error);
            let _ = log.write_string(format!("vreg registry load failed: {:?}", error));
            return Err(error);
        },
    };

    println!("[INFO] vreg publishing service");
    log.write_string("vreg publishing service".into())?;

    let accept = match publish_service() {
        Ok(accept) => accept,
        Err(error) => {
            println!("[ERROR] vreg service publish failed: {:?}", error);
            let _ = log.write_string(format!("vreg service publish failed: {:?}", error));
            return Err(error);
        }
    };

    println!("[INFO] vreg online");
    log.write_string("vreg online".into())?;

    let mut waiter = Waiter::new().readable(accept.handle());

    loop {
        waiter.wait()?;
        if waiter.ready(0) {
            match recv_vreg_accept(&accept) {
                Ok(session) => {
                    if let Err(error) = spawn_vreg_session(session, registry.clone()) {
                        let _ = log.write_string(format!("failed to spawn vreg session: {:?}", error));
                    }
                },
                Err(error) => {
                    let _ = log.write_string(format!("invalid vreg accept message: {:?}", error));
                },
            }
        }
        waiter.clear();
    }
}
