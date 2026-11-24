use vespertine_abi::UserID;
use vespertine_abi::app::hesper::{
    AppIoMode,
    AppIoModes,
    HESPER_STATUS_INVALID_REQUEST,
    HESPER_STATUS_LAUNCH_FAILED,
    HESPER_STATUS_NOT_FOUND,
    HESPER_STATUS_OK,
};
use vespertine_abi::tag::{CAP_FRAMEBUFFER, CAP_PROCMAN};
use vrt::syscall::{
    sys_close,
    sys_yield,
};
use vstd::{Exec, Process};
use vstd::broker::Broker;
use vstd::fs::resolve;
use vstd::hesper::{
    CapabilityOffer,
    ExecuteRequest,
    HesperRequest,
    send_app_metadata_response,
    send_execute_response,
};
use vstd::log::SystemLog;
use vstd::portal::{
    accept_handle,
    offer_handle,
    revoke_offer,
};
use vstd::prelude::*;
use vstd::vreg::{
    ResolvedApplication,
    VRegistryClient,
};

use crate::meta::load_manifest;
use crate::policy::{
    CapabilityPolicy, LaunchPolicy, PolicyStore,
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
    let context = |error: Error| error.context("cannot connect to VReg");
    for _ in 0..100 {
        match VRegistryClient::connect() {
            Ok(client) => return Ok(client),
            Err(error) if error.kind == ErrorKind::NotFound => sys_yield(),
            Err(error) => return Err(context(error)),
        }
    }
    Err(Error::unknown("VReg service did not appear".into()))
}

fn resolve_request(name: &str) -> Result<ResolvedApplication, Error> {
    let mut registry = connect_registry()?;
    registry.resolve(name)
        .map_err(|error| error.context(format_args!("cannot resolve application '{name}' through VReg")))
}

pub fn handle_request(
    socket: &Socket, request: HesperRequest, log: &SystemLog, policy: &PolicyStore, caller_user: UserID,
) -> Result<(), Error> {
    match request {
        HesperRequest::AppMetadata { request_id, request } => match resolve_request(&request.app_name) {
            Ok(app) => send_app_metadata_response(
                socket,
                request_id,
                HESPER_STATUS_OK,
                app.input,
                app.modes,
                app.default_mode,
                &app.app_id,
                &app.display_name,
            ),
            Err(error) => {
                let status = if error.kind == ErrorKind::NotFound {
                    HESPER_STATUS_NOT_FOUND
                } else {
                    HESPER_STATUS_LAUNCH_FAILED
                };
                send_app_metadata_response(socket, request_id, status, AppIoMode::Any, AppIoModes::new(), AppIoMode::Any, "", "")?;
                return Err(error);
            }
        },
        HesperRequest::Execute { request_id, request } => {
            log.write_string(format!("execute requested: {} with {} arguments", request.app_name, request.arguments.len()))?;
            handle_execute(socket, request_id, request, log, policy, caller_user)
        }
    }
}

struct PreparedLaunch {
    app: ResolvedApplication,
    policy: LaunchPolicy,
}

struct PrepareError {
    status: u32,
    error: Error,
    public_message: Option<&'static str>,
}

impl PrepareError {
    fn invalid(error: Error, public_message: Option<&'static str>) -> Self {
        Self { status: HESPER_STATUS_INVALID_REQUEST, error, public_message }
    }
}

fn prepare_launch(name: &str, mode: AppIoMode, policies: &PolicyStore) -> Result<PreparedLaunch, PrepareError> {
    let app = resolve_request(&name).map_err(|error| {
        let status = if error.kind == ErrorKind::NotFound {
            HESPER_STATUS_NOT_FOUND
        } else {
            HESPER_STATUS_LAUNCH_FAILED
        };
        PrepareError { status, error, public_message: None }
    })?;

    let manifest = load_manifest(&app.bundle)
        .map_err(|e| PrepareError::invalid(e, Some("Application manifest is invalid")))?;

    if manifest.application.id != app.app_id {
        return Err(PrepareError::invalid(
                Error::access_denied("Registry app ID does not match bundle manifest".into()), 
                Some("Application registry app ID does not match bundle manifest"),
        ));
    }

    let policy = policies.resolve(&app.app_id, &manifest)
        .map_err(|e| PrepareError::invalid(e, None))?;

    if mode == AppIoMode::Any || !app.modes.contains_mode(mode) {
        return Err(PrepareError::invalid(
                Error::invalid_argument("Application does not support requested launch mode".into()), 
                None,
        ));
    }

    Ok(PreparedLaunch { app, policy })
}

pub struct LaunchHandles {
    pub source: HandleID,
    pub sink: HandleID,
    pub cwd: HandleID,
}

fn spawn_application(launch: PreparedLaunch, user: UserID, handles: LaunchHandles, args: &[String], capabilites: &[AcceptedCapability]) -> Result<Process, Error> {
    let PreparedLaunch { app, policy } = launch;
    let binary_path = format!("{}/bin/{}", app.bundle, app.binary);
    let mut exec = Exec::open(&Path::new(&binary_path), app.binary)?
        .start_suspended()
        .args(args)
        .source(handles.source)
        .sink(handles.sink)
        .cwd(handles.cwd, policy.cwd_rights)
        .namespace_rights(policy.namespace_rights)
        .user(user);

    for grant in capabilites {
        exec = exec.grant_new(grant.handle.handle(), grant.policy.capability, grant.policy.rights)?;
    }

    exec.spawn()
}

fn handle_execute(socket: &Socket, request_id: u32, request: ExecuteRequest, log: &SystemLog, policy: &PolicyStore, caller_user: UserID) -> Result<(), Error> {
    let launch = match prepare_launch(&request.app_name, request.mode, policy) {
        Ok(launch) => launch,
        Err(e) => {
            let message = e.public_message.map(String::from).unwrap_or_else(|| e.error.to_string());
            send_execute_response(socket, request_id, e.status, None, &message)?;
            return Err(e.error);
        }
    };
    let policy = &launch.policy;

    let session = socket.handle();
    let accepted = (|| {
        let source = AcceptedHandle::accept(session, request.source_offer, AccessRights::READ)
            .map_err(|error| error.context("cannot accept standard-input offer"))?;
        let sink = AcceptedHandle::accept(session, request.sink_offer, AccessRights::WRITE)
            .map_err(|error| error.context("cannot accept standard-output offer"))?;
        let cwd = AcceptedHandle::accept(session, request.cwd_offer, policy.cwd_rights)
            .map_err(|error| error.context("cannot accept working-directory offer"))?;
        Ok::<_, Error>((source, sink, cwd))
    })();

    let (source, sink, cwd) = match accepted {
        Ok(handles) => handles,
        Err(error) => {
            send_execute_response(socket, request_id, HESPER_STATUS_INVALID_REQUEST, None, &error.to_string())?;
            return Err(error);
        }
    };

    let mut system_capabilities = Vec::new();
    let mut caller_capabilities = Vec::new();

    for capability in &policy.capabilities {
        match request_system_capability(*capability) {
            Ok(Some(capability)) => system_capabilities.push(capability),
            Ok(None) => caller_capabilities.push(*capability),
            Err(error) => {
                let _ = send_execute_response(
                    socket,
                    request_id,
                    HESPER_STATUS_INVALID_REQUEST,
                    None,
                    "application launch policy denied the request",
                );

                return Err(error);
            }
        }
    }

    let mut accepted_capabilities = match accept_capabilities(session, &request.capability_offers, &caller_capabilities) {
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

    accepted_capabilities.extend(system_capabilities);

    let spawn_result = spawn_application(
        launch, 
        caller_user, 
        LaunchHandles {
            source: source.handle(),
            sink: sink.handle(),
            cwd: cwd.handle(),
        }, 
        &request.arguments, 
        &accepted_capabilities,
    );

    let process = match spawn_result {
        Ok(process) => process,

        Err(error) => {
            send_execute_response(socket, request_id, HESPER_STATUS_LAUNCH_FAILED, None, &error.to_string())?;
            return Err(error);
        }
    };

    let process_offer = match offer_handle(session, process.handle(), AccessRights::READ | AccessRights::WRITE) {
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

pub fn spawn_local(name: &str, mode: AppIoMode, policies: &PolicyStore, user: UserID, handles: LaunchHandles) -> Result<Process, Error> {
    let launch = prepare_launch(name, mode, policies).map_err(|e| e.error)?;
    let mut capabilities = Vec::new();

    for &grant in &launch.policy.capabilities {
        let capability = request_system_capability(grant)?.ok_or_else(|| {
            Error::invalid_argument(format!("Local launch of '{name}' requires a caller-supplied capability"))
        })?;
        capabilities.push(capability);
    }

    spawn_application(launch, user, handles, &[], &capabilities)
}

fn send_launcher_invalid_request(sock: &Socket, id: u32) {
    let _ = send_app_metadata_response(&sock, id, HESPER_STATUS_INVALID_REQUEST, AppIoMode::Any, AppIoModes::new(), AppIoMode::Any, "", "");
}

fn request_system_capability(policy: CapabilityPolicy) -> Result<Option<AcceptedCapability>, Error> {
    match policy.capability {
        CAP_PROCMAN => {
            let broker_handle = resolve(&Path::new("/System/Services/ProcManager"), AccessRights::READ)?;
            let broker = Broker::from_handle(broker_handle);
            let handle = broker.request(policy.capability, policy.rights)?;

            Ok(Some(AcceptedCapability { handle: AcceptedHandle { handle }, policy }))
        },
        CAP_FRAMEBUFFER => {
            let handle = resolve(&Path::new("/Devices/Framebuffer"), policy.rights)?;
            Ok(Some(AcceptedCapability { handle: AcceptedHandle { handle }, policy }))
        },
        _ => Ok(None),
    }
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
