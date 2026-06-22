use alloc::format;

use vespertine_abi::{AccessRights, HandleID};
use vespertine_abi::app::hesper::{
    AppIoMode, HESPER_STATUS_INVALID_REQUEST, HESPER_STATUS_LAUNCH_FAILED, HESPER_STATUS_NOT_FOUND, HESPER_STATUS_NOT_IMPLEMENTED, HESPER_STATUS_OK
};
use vespertine_rt::syscall::sys_close;
use vespertine_std::fs::Path;
use vespertine_std::hesper::{
    ExecuteRequest, HesperRequest, decode_io_mode_string, send_app_metadata_response, send_execute_response
};
use vespertine_std::log::SystemLog;
use vespertine_std::portal::{accept_handle, offer_handle, revoke_offer};
use vespertine_std::socket::Socket;
use vespertine_std::{
    Error, Exec, Write
};

use crate::meta::get_metadata;

struct AcceptedHandle {
    handle: HandleID
}

impl AcceptedHandle {
    fn accept(session: HandleID, offer_id: usize, rights: AccessRights) -> Result<Self, Error> {
        let handle = accept_handle(session, offer_id, rights)?;
        Ok(Self { handle })
    }

    fn handle(&self) -> HandleID {
        self.handle
    }
}

impl Drop for AcceptedHandle {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}

pub fn handle_request(socket: &Socket, request: HesperRequest, log: &SystemLog) -> Result<(), Error> {
    match request {
        HesperRequest::AppMetadata { request_id, request } => match get_metadata(&request.app_name) {
            Ok(metadata) => {
                let input = match decode_io_mode_string(&metadata.io.input) {
                    Ok(value) => value,
                    Err(error) => {
                        send_launcher_invalid_request(socket, request_id);
                        return Err(error);
                    }
                };
                let output = match decode_io_mode_string(&metadata.io.output) {
                    Ok(value) => value,
                    Err(error) => {
                        send_launcher_invalid_request(socket, request_id);
                        return Err(error);
                    }
                };
                send_app_metadata_response(
                    socket,
                    request_id,
                    HESPER_STATUS_OK,
                    input,
                    output,
                    &metadata.application.id,
                    &metadata.application.name,
                )
            }
            Err(error) => {
                log.write_string(format!("metadata lookup failed for {}: {:?}", request.app_name, error))?;
                send_launcher_not_found(socket, request_id);
                return Err(error);
            }
        },
        HesperRequest::Execute { request_id, request } => {
            log.write_string(format!("execute requested: {} with {} arguments", request.app_name, request.arguments.len()))?;
            handle_execute(socket, request_id, request, log)
        },
    }
}


fn handle_execute(socket: &Socket, request_id: u32, request: ExecuteRequest, log: &SystemLog) -> Result<(), Error> {
    let metadata = match get_metadata(&request.app_name) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = send_execute_response(
                socket,
                request_id,
                HESPER_STATUS_NOT_FOUND,
                None,
                "application was not found",
            );
            return Err(error);
        },
    };
    let session = socket.handle();
    let accepted = (|| {
        let source = AcceptedHandle::accept(
            session,
            request.source_offer,
            AccessRights::READ,
        )?;
        let sink = AcceptedHandle::accept(
            session,
            request.sink_offer,
            AccessRights::WRITE,
        )?;
        let cwd = AcceptedHandle::accept(
            session,
            request.cwd_offer,
            AccessRights::TRAVERSE,
        )?;
        Ok::<_, Error>((source, sink, cwd))
    })();

    let (source, sink, cwd) = match accepted {
        Ok(handles) => handles,
        Err(error) => {
            let _ = send_execute_response(
                socket,
                request_id,
                HESPER_STATUS_INVALID_REQUEST,
                None,
                "invalid launch object offer",
            );
            return Err(error);
        },
    };

    log.write_string(format!("launching {} as {}", request.app_name, metadata.application.binary))?;

    let binary_path = format!("/Programs/{}.app/bin/{}", request.app_name, metadata.application.binary);

    let process = match Exec::open(&Path::new(&binary_path), metadata.application.binary)?
        .args(&request.arguments)
        .source(source.handle())
        .sink(sink.handle())
        .cwd(cwd.handle(), AccessRights::TRAVERSE)
        .root_rights(AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE | 
            AccessRights::TRAVERSE | AccessRights::LIST | AccessRights::REMOVE)
        .spawn()
    {
        Ok(process) => process,
        Err(error) => {
            let _ = send_execute_response(
                socket,
                request_id,
                HESPER_STATUS_LAUNCH_FAILED,
                None,
                "failed to spawn application",
            );
            return Err(error);
        },
    };

    let process_offer = match offer_handle(session, process.handle(), AccessRights::READ) {
        Ok(offer) => offer,
        Err(error) => {
            let _ = send_execute_response(
                socket,
                request_id,
                HESPER_STATUS_LAUNCH_FAILED,
                None,
                "failed to return process capability",
            );
            return Err(error);
        },
    };

    if let Err(error) = send_execute_response(socket, request_id, HESPER_STATUS_OK, Some(process_offer), "") {
        // the client never learned the offer id.
        let _ = revoke_offer(
            session,
            process_offer,
        );
        return Err(error);
    }
    Ok(())
}

fn send_launcher_not_found(sock: &Socket, id: u32) {
    let _ = send_app_metadata_response(&sock, id, HESPER_STATUS_NOT_FOUND, AppIoMode::Any, AppIoMode::Any, "", "");
}

fn send_launcher_invalid_request(sock: &Socket, id: u32) {
    let _ = send_app_metadata_response(&sock, id, HESPER_STATUS_INVALID_REQUEST, AppIoMode::Any, AppIoMode::Any, "", "");
}
