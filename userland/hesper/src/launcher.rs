use alloc::format;
use alloc::vec::Vec;

use vespertine_abi::app::hesper::{
    AppIoMode, AppIoModes, HESPER_STATUS_INVALID_REQUEST, HESPER_STATUS_LAUNCH_FAILED, HESPER_STATUS_NOT_FOUND, HESPER_STATUS_NOT_IMPLEMENTED, HESPER_STATUS_OK
};
use vespertine_abi::{
    AccessRights,
    HandleID,
};
use vespertine_rt::syscall::{sys_close, sys_yield};
use vespertine_std::fs::Path;
use vespertine_std::hesper::{
    CapabilityOffer, ExecuteRequest, HesperRequest, decode_io_mode_string, decode_io_modes_strings, send_app_metadata_response, send_execute_response
};
use vespertine_std::log::SystemLog;
use vespertine_std::portal::{
    accept_handle,
    offer_handle,
    revoke_offer,
};
use vespertine_std::socket::Socket;
use vespertine_std::vreg::{ResolvedApplication, VRegistryClient};
use vespertine_std::{
    Error,
    Exec,
    Write,
};

use crate::meta::{
    AppManifest,
    EntrypointMetadata,
    load_manifest,
    select_entrypoint,
};
use crate::policy::{
    CapabilityPolicy,
    PolicyStore,
};

struct AcceptedHandle {
    handle: HandleID,
}

impl AcceptedHandle {
    fn accept(session: HandleID, offer_id: usize, rights: AccessRights) -> Result<Self, Error> {
        let handle = accept_handle(session, offer_id, rights)?;
        Ok(Self { handle })
    }

    fn handle(&self) -> HandleID { self.handle }
}

impl Drop for AcceptedHandle {
    fn drop(&mut self) { let _ = sys_close(self.handle); }
}

struct AcceptedCapability {
    handle: AcceptedHandle,
    policy: CapabilityPolicy,
}

fn connect_registry() -> Result<VRegistryClient, Error> {
    for _ in 0..100 {
        match VRegistryClient::connect() {
            Ok(client) => return Ok(client),
            Err(_) => sys_yield(),
        }
    }
    VRegistryClient::connect()
}

fn resolve_request(name: &str) -> Result<ResolvedApplication, Error> {
    let mut registry = connect_registry()?;
    registry.resolve(name)
}

pub fn handle_request(socket: &Socket, request: HesperRequest, log: &SystemLog, policy: &PolicyStore) -> Result<(), Error> {
    match request {
        HesperRequest::AppMetadata { request_id, request } => match resolve_request(&request.app_name) {
            Ok(app) => {
                send_app_metadata_response(
                    socket,
                    request_id,
                    HESPER_STATUS_OK,
                    app.input,
                    app.modes,
                    app.default_mode,
                    &app.app_id,
                    &app.display_name,
                )
            },
            Err(error) => {
                log.write_string(format!("metadata lookup failed for {}: {:?}", request.app_name, error))?;
                send_launcher_not_found(socket, request_id);
                return Err(error);
            }
        },
        HesperRequest::Execute { request_id, request } => {
            log.write_string(format!("execute requested: {} with {} arguments", request.app_name, request.arguments.len()))?;
            handle_execute(socket, request_id, request, log, policy)
        },
    }
}

fn handle_execute(socket: &Socket, request_id: u32, request: ExecuteRequest, log: &SystemLog, policy: &PolicyStore) -> Result<(), Error> {
    let app = match resolve_request(&request.app_name) {
        Ok(app) => app,
        Err(error) => {
            let _ = send_execute_response(socket, request_id, HESPER_STATUS_NOT_FOUND, None, "application was not found");
            return Err(error);
        }
    };
    
    let metadata = match load_manifest(&app.bundle) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = send_execute_response(socket, request_id, HESPER_STATUS_INVALID_REQUEST, None, "application manifest is invalid");
            return Err(error);
        }
    };
    
    if metadata.application.id != app.app_id {
        let _ = send_execute_response(socket, request_id, HESPER_STATUS_INVALID_REQUEST, None, "application registry does not match bundle manifest");
        return Err(Error::access_denied("registry app ID does not match bundle manifest".into()));
    }
    
    let policy = match policy.resolve(&app.app_id, &metadata) {
        Ok(policy) => policy,
        Err(error) => {
            let _ = send_execute_response(
                socket,
                request_id,
                HESPER_STATUS_INVALID_REQUEST,
                None,
                "application launch policy denied the request",
            );
            return Err(error);
        },
    };
    
    if request.mode == AppIoMode::Any || !app.modes.contains_mode(request.mode) {
        let _ = send_execute_response(socket, request_id, HESPER_STATUS_INVALID_REQUEST, None, "application does not support requested launch mode");
        return Err(Error::invalid_argument("application does not support requested launch mode".into()));
    }

    let session = socket.handle();
    let accepted = (|| {
        let source = AcceptedHandle::accept(session, request.source_offer, AccessRights::READ)?;
        let sink = AcceptedHandle::accept(session, request.sink_offer, AccessRights::WRITE)?;
        let cwd = AcceptedHandle::accept(session, request.cwd_offer, policy.cwd_rights)?;
        Ok::<_, Error>((source, sink, cwd))
    })();

    let (source, sink, cwd) = match accepted {
        Ok(handles) => handles,
        Err(error) => {
            let _ = send_execute_response(socket, request_id, HESPER_STATUS_INVALID_REQUEST, None, "invalid launch object offer");
            return Err(error);
        }
    };

    let accepted_capabilities = match accept_capabilities(session, &request.capability_offers, &policy.capabilities) {
        Ok(capabilities) => capabilities,

        Err(error) => {
            let _ = send_execute_response(
                socket,
                request_id,
                HESPER_STATUS_INVALID_REQUEST,
                None,
                "required launch capability was not offered",
            );

            return Err(error);
        }
    };

    log.write_string(format!("launching {}:{} as {}", app.app_id, app.entrypoint, app.binary))?;
    
    let binary_path = format!("{}/bin/{}", app.bundle, app.binary);
    
    let spawn_result = (|| { let mut exec = Exec::open(&Path::new(&binary_path), app.binary)?
            .args(&request.arguments)
            .source(source.handle())
            .sink(sink.handle())
            .cwd(cwd.handle(), policy.cwd_rights)
            .root_rights(policy.root_rights);

        for capability in &accepted_capabilities {
            exec = exec.grant_new(capability.handle.handle(), capability.policy.capability, capability.policy.rights)?;
        }
        exec.spawn()
    })();

    let process = match spawn_result {
        Ok(process) => process,

        Err(error) => {
            let _ = send_execute_response(socket, request_id, HESPER_STATUS_LAUNCH_FAILED, None, "failed to spawn application");
            return Err(error);
        }
    };

    let process_offer = match offer_handle(session, process.handle(), AccessRights::READ) {
        Ok(offer) => offer,
        Err(error) => {
            let _ = send_execute_response(socket, request_id, HESPER_STATUS_LAUNCH_FAILED, None, "failed to return process capability");
            return Err(error);
        }
    };

    if let Err(error) = send_execute_response(socket, request_id, HESPER_STATUS_OK, Some(process_offer), "") {
        // the client never learned the offer id.
        let _ = revoke_offer(session, process_offer);
        return Err(error);
    }
    Ok(())
}

fn send_launcher_not_found(sock: &Socket, id: u32) {
    let _ = send_app_metadata_response(&sock, id, HESPER_STATUS_NOT_FOUND, 
        AppIoMode::Any, AppIoModes::new(), AppIoMode::Any, "", "");
}

fn send_launcher_invalid_request(sock: &Socket, id: u32) {
    let _ = send_app_metadata_response(&sock, id, HESPER_STATUS_INVALID_REQUEST, 
        AppIoMode::Any, AppIoModes::new(), AppIoMode::Any, "", "");
}

fn accept_capabilities(
    session: HandleID, offers: &[CapabilityOffer], policies: &[CapabilityPolicy],
) -> Result<Vec<AcceptedCapability>, Error> {
    let mut accepted = Vec::with_capacity(policies.len());

    for policy in policies {
        let offer = offers
            .iter()
            .find(|offer| offer.capability == policy.capability)
            .ok_or_else(|| Error::access_denied("required capability was not offered".into()))?;

        let handle = AcceptedHandle::accept(session, offer.offer_id, policy.rights)?;

        accepted.push(AcceptedCapability { handle, policy: *policy });
    }

    Ok(accepted)
}
