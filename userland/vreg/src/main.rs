#![no_std]
#![no_main]

extern crate alloc;

mod registry;

use vabi::app::vreg::VREG_STATUS_OK;
use vabi::tag::CAP_VREG_CONNECT;
use vrt::syscall::sys_close;
use vrt::thread as rt_thread;
use vstd::fs::{
    link_object,
    resolve,
};
use vstd::log::SystemLog;
use vstd::portal::PortalFactory;
use vstd::prelude::*;
use vstd::proc::Waiter;
use vstd::sync::RwLock;
use vstd::vreg::{
    VRegistryRequest,
    recv_vreg_request,
    send_list_end,
    send_list_entry,
    send_reload_response,
    send_resolve_response,
    status_from_error,
};

use crate::registry::AppRegistry;

type SharedRegistry = Arc<RwLock<AppRegistry>>;

fn recv_vreg_accept(socket: &Socket) -> Result<HandleID, Error> {
    let mut bytes = [0u8; 8];
    socket.read_exact(&mut bytes)?;
    Ok(HandleID(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize))
}

fn handle_request(socket: &Socket, request: VRegistryRequest, registry: &SharedRegistry, log: &SystemLog) -> Result<(), Error> {
    match request {
        VRegistryRequest::Resolve { request_id, name } => {
            let result = {
                let registry = registry.read();
                registry.resolve(&name)
            };
            match result {
                Ok(app) => send_resolve_response(socket, request_id, VREG_STATUS_OK, Some(&app)),
                Err(error) => {
                    let _ = log.write_string(format!("vreg resolve failed for {}: {:?}", name, error));
                    send_resolve_response(socket, request_id, status_from_error(&error), None)?;
                    Err(error)
                }
            }
        }
        VRegistryRequest::List { request_id } => {
            let entries = {
                let registry = registry.read();
                registry.list()
            };
            let entries = match entries {
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
        }
        VRegistryRequest::Reload { request_id } => {
            let loaded = match AppRegistry::load() {
                Ok(registry) => registry,
                Err(error) => {
                    let _ = log.write_string(format!("vreg reload failed: {:?}", error));
                    send_reload_response(socket, request_id, status_from_error(&error))?;
                    return Err(error);
                }
            };
            registry.replace(loaded);
            let _ = log.write_string("vreg registry reloaded".into());
            send_reload_response(socket, request_id, VREG_STATUS_OK)
        }
    }
}

fn spawn_vreg_session(handle: HandleID, registry: SharedRegistry) -> Result<(), Error> {
    rt_thread::spawn(move || {
        let log = SystemLog::connect();
        let socket = Socket::from_handle(handle);
        loop {
            let request = match recv_vreg_request(&socket) {
                Ok(request) => request,
                Err(error) if error.kind == ErrorKind::EndOfStream => {
                    break;
                }
                Err(error) => {
                    let _ = log.write_string(format!("vreg session failed: {:?}", error));
                    break;
                }
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

#[vapp::main]
fn main() -> Result<(), Error> {
    let log = SystemLog::connect();

    println!("[INFO] vreg starting");
    log.write_string("vreg starting".into())?;

    println!("[INFO] vreg loading registry");
    log.write_string("vreg loading registry".into())?;

    let registry = match AppRegistry::load() {
        Ok(registry) => Arc::new(RwLock::new(registry)),
        Err(error) => {
            println!("[ERROR] vreg registry load failed: {:?}", error);
            let _ = log.write_string(format!("vreg registry load failed: {:?}", error));
            return Err(error);
        }
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
                }
                Err(error) => {
                    let _ = log.write_string(format!("invalid vreg accept message: {:?}", error));
                }
            }
        }

        waiter.clear();
    }
}
