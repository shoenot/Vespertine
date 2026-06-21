use alloc::format;

use vespertine_abi::app::hesper::{
    AppIoMode,
    HESPER_STATUS_INVALID_REQUEST,
    HESPER_STATUS_NOT_FOUND,
    HESPER_STATUS_OK,
};
use vespertine_std::hesper::{
    HesperRequest,
    decode_io_mode_string,
    send_app_metadata_response,
    send_execute_response,
};
use vespertine_std::log::SystemLog;
use vespertine_std::socket::Socket;
use vespertine_std::{
    Error,
    Write,
};

use crate::meta::get_metadata;

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
            send_execute_response(socket, request_id, HESPER_STATUS_OK, "unimplemented rn")
        }
    }
}

fn send_launcher_not_found(sock: &Socket, id: u32) {
    let _ = send_app_metadata_response(&sock, id, HESPER_STATUS_NOT_FOUND, AppIoMode::Any, AppIoMode::Any, "", "");
}

fn send_launcher_invalid_request(sock: &Socket, id: u32) {
    let _ = send_app_metadata_response(&sock, id, HESPER_STATUS_INVALID_REQUEST, AppIoMode::Any, AppIoMode::Any, "", "");
}
