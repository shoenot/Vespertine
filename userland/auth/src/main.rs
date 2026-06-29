#![no_std]
#![no_main]

extern crate alloc;
use vstd::prelude::*;

mod accounts;

use vabi::app::auth::AUTH_STATUS_OK;
use vabi::tag::CAP_AUTH_CONNECT;
use vrt::syscall::sys_close;
use vrt::thread as rt_thread;
use vstd::auth::{
    AuthRequest,
    recv_auth_request,
    send_account_response,
    status_from_error,
};
use vstd::fs::{
    link_object,
    resolve,
};
use vstd::log::SystemLog;
use vstd::portal::PortalFactory;
use vstd::proc::Waiter;

use crate::accounts::AccountStore;

fn recv_auth_accept(socket: &Socket) -> Result<HandleID, Error> {
    let mut bytes = [0u8; 8];
    socket.read_exact(&mut bytes)?;
    Ok(HandleID(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize))
}

fn handle_request(socket: &Socket, request: AuthRequest, accounts: &AccountStore, log: &SystemLog) -> Result<(), Error> {
    match request {
        AuthRequest::DefaultUser { request_id } => match accounts.default_user().and_then(|account| account.info()) {
            Ok(account) => send_account_response(socket, request_id, AUTH_STATUS_OK, Some(&account)),
            Err(error) => {
                let _ = log.write_string(format!("auth default user lookup failed: {:?}", error));
                send_account_response(socket, request_id, status_from_error(&error), None)?;
                Err(error)
            }
        },
        AuthRequest::LookupId { request_id, user } => {
            let _ = log.write_string(format!("auth lookup id {} start", user.0));
            match accounts.by_id(user).and_then(|account| account.info()) {
                Ok(account) => {
                    let result = send_account_response(socket, request_id, AUTH_STATUS_OK, Some(&account));
                    let _ = log.write_string(format!("auth lookup id {} response sent", user.0));
                    result
                }
                Err(error) => {
                    let _ = log.write_string(format!("auth user id lookup failed for {}: {:?}", user.0, error));
                    send_account_response(socket, request_id, status_from_error(&error), None)?;
                    Err(error)
                }
            }
        }
        AuthRequest::LookupName { request_id, name } => match accounts.by_name(&name).and_then(|account| account.info()) {
            Ok(account) => send_account_response(socket, request_id, AUTH_STATUS_OK, Some(&account)),
            Err(error) => {
                let _ = log.write_string(format!("auth user name lookup failed for {}: {:?}", name, error));
                send_account_response(socket, request_id, status_from_error(&error), None)?;
                Err(error)
            }
        },
    }
}

fn spawn_auth_session(handle: HandleID, accounts: Arc<AccountStore>) -> Result<(), Error> {
    rt_thread::spawn(move || {
        let log = SystemLog::connect();
        let socket = Socket::from_handle(handle);

        loop {
            let request = match recv_auth_request(&socket) {
                Ok(request) => request,
                Err(error) if error.kind == ErrorKind::EndOfStream => {
                    break;
                }
                Err(error) => {
                    let _ = log.write_string(format!("auth session failed: {:?}", error));
                    break;
                }
            };

            if let Err(error) = handle_request(&socket, request, &accounts, &log) {
                if error.kind != ErrorKind::NotFound {
                    let _ = log.write_string(format!("auth request failed: {:?}", error));
                }
            }
        }
    })
    .map(|_| ())
    .map_err(Error::from)
}

fn publish_service() -> Result<Socket, Error> {
    let portal_factory = PortalFactory::request()?;
    let (portal, accept) = portal_factory.create(CAP_AUTH_CONNECT, AccessRights::READ | AccessRights::WRITE)?;

    let services = resolve(&Path::new("/System/Services"), AccessRights::CREATE)?;
    link_object(services, "Auth", portal)?;

    sys_close(portal).map_err(Error::from)?;
    sys_close(services).map_err(Error::from)?;

    Ok(Socket::from_handle(accept))
}

#[vapp::main]
fn main() -> Result<(), Error> {
    let log = SystemLog::connect();

    println!("[INFO] auth starting");
    log.write_string("auth starting".into())?;

    println!("[INFO] auth loading accounts");
    log.write_string("auth loading accounts".into())?;

    let accounts = match AccountStore::load() {
        Ok(accounts) => Arc::new(accounts),
        Err(error) => {
            println!("[ERROR] auth account load failed: {:?}", error);
            let _ = log.write_string(format!("auth account load failed: {:?}", error));
            return Err(error);
        }
    };

    println!("[INFO] auth publishing service");
    log.write_string("auth publishing service".into())?;

    let accept = match publish_service() {
        Ok(accept) => accept,
        Err(error) => {
            println!("[ERROR] auth service publish failed: {:?}", error);
            let _ = log.write_string(format!("auth service publish failed: {:?}", error));
            return Err(error);
        }
    };

    println!("[INFO] auth online");
    log.write_string("auth online".into())?;

    let mut waiter = Waiter::new().readable(accept.handle());

    loop {
        waiter.wait()?;

        if waiter.ready(0) {
            match recv_auth_accept(&accept) {
                Ok(session) => {
                    if let Err(error) = spawn_auth_session(session, accounts.clone()) {
                        let _ = log.write_string(format!("failed to spawn auth session: {:?}", error));
                    }
                }
                Err(error) => {
                    let _ = log.write_string(format!("invalid auth accept message: {:?}", error));
                }
            }
        }
        waiter.clear();
    }
}
