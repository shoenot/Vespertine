extern crate alloc;

use alloc::format;
use alloc::string::{
    String,
    ToString,
};
use alloc::vec::Vec;

use vespertine_abi::app::hesper::*;
use vespertine_abi::tag::CAP_LAUNCHER_CONNECT;
use vespertine_abi::{
    AccessRights,
    CapabilityID,
    HandleID,
};

use crate::broker::Broker;
use crate::fs::{
    Path,
    resolve,
};
use crate::payload::*;
use crate::portal::{
    PortalOfferId,
    accept_handle,
    offer_handle,
    revoke_offer,
};
use crate::socket::Socket;
use crate::{
    Error,
    Process,
    env,
};

const MAX_CAPABILITY_OFFERS: usize = 32;
const MAX_APP_NAME: usize = 256;
const MAX_ARGUMENTS: usize = 128;
const MAX_ARGUMENT_LENGTH: usize = 4096;

#[derive(Debug)]
pub struct AppMetadataRequest {
    pub app_name: String,
}

#[derive(Debug)]
pub struct AppMetadataResponse {
    pub status: u32,
    pub input: AppIoMode,
    pub default_mode: AppIoMode,
    pub modes: AppIoModes,
    pub app_id: String,
    pub display_name: String,
}

#[derive(Debug)]
pub struct ExecuteRequest {
    pub app_name: String,
    pub arguments: Vec<String>,
    pub mode : AppIoMode,

    pub source_offer: PortalOfferId,
    pub sink_offer: PortalOfferId,
    pub cwd_offer: PortalOfferId,

    pub capability_offers: Vec<CapabilityOffer>,
}

#[derive(Debug)]
pub struct ExecuteResponse {
    pub status: u32,
    pub process_offer: Option<PortalOfferId>,
    pub message: String,
}

#[derive(Debug)]
pub enum HesperRequest {
    AppMetadata { request_id: u32, request: AppMetadataRequest },
    Execute { request_id: u32, request: ExecuteRequest },
}

#[derive(Debug)]
pub enum HesperResponse {
    AppMetadata { request_id: u32, response: AppMetadataResponse },
    Execute { request_id: u32, response: ExecuteResponse },
}

#[derive(Debug, Clone, Copy)]
pub struct CapabilityOffer {
    pub capability: CapabilityID,
    pub offer_id: PortalOfferId,
}

impl<'a> PayloadReader<'a> {
    fn read_capability_id(&mut self) -> Result<CapabilityID, Error> {
        let raw = self.read_u64()?;
        let value = usize::try_from(raw).map_err(|_| Error::invalid_argument("capability ID is out of range".into()))?;
        if value == 0 {
            return Err(Error::invalid_argument("capability ID cannot be zero".into()));
        }
        Ok(CapabilityID(value))
    }
}

fn read_hesper_header(reader: &mut PayloadReader<'_>) -> Result<u32, Error> {
    let _flags = reader.read_u16()?;
    let request_id = reader.read_u32()?;
    Ok(request_id)
}

pub fn decode_io_mode(value: u8) -> Result<AppIoMode, Error> {
    match value {
        0 => Ok(AppIoMode::Any),
        1 => Ok(AppIoMode::Text),
        2 => Ok(AppIoMode::Typed),
        3 => Ok(AppIoMode::Terminal),
        _ => Err(Error::invalid_argument("invalid app I/O mode".into())),
    }
}

pub fn decode_io_mode_string(value: &str) -> Result<AppIoMode, Error> {
    match value {
        "any" => Ok(AppIoMode::Any),
        "text" => Ok(AppIoMode::Text),
        "typed" => Ok(AppIoMode::Typed),
        "terminal" => Ok(AppIoMode::Terminal),
        _ => Err(Error::invalid_argument("invalid app I/O mode".into())),
    }
}

fn decode_metadata_request(reader: &mut PayloadReader<'_>) -> Result<AppMetadataRequest, Error> {
    let app_name = reader.read_string(MAX_APP_NAME, "application name")?;
    Ok(AppMetadataRequest { app_name })
}

fn decode_execute_request(reader: &mut PayloadReader<'_>) -> Result<ExecuteRequest, Error> {
    let app_name = reader.read_string(MAX_APP_NAME, "application name")?;
    let argument_count = reader.read_u32()? as usize;
    if argument_count > MAX_ARGUMENTS {
        return Err(Error::invalid_argument("too many application arguments".into()));
    }
    let mut arguments = Vec::with_capacity(argument_count);
    for _ in 0..argument_count {
        arguments.push(reader.read_string(MAX_ARGUMENT_LENGTH, "application argument")?);
    }

    let mode = decode_io_mode(reader.read_u8()?)?;
    let _flags = reader.read_u8()?;
    let _reserved = reader.read_u16()?;

    let source_offer = reader.read_offer_id("source offer")?;
    let sink_offer = reader.read_offer_id("sink offer")?;
    let cwd_offer = reader.read_offer_id("cwd offer")?;

    let capability_count = reader.read_u32()? as usize;

    if capability_count > MAX_CAPABILITY_OFFERS {
        return Err(Error::invalid_argument("too many capability offers".into()));
    }

    let mut capability_offers = Vec::with_capacity(capability_count);

    for _ in 0..capability_count {
        let capability = reader.read_capability_id()?;
        let offer_id = reader.read_offer_id("capability offer")?;

        if capability_offers.iter().any(|offer: &CapabilityOffer| offer.capability == capability) {
            return Err(Error::invalid_argument("duplicate capability offer".into()));
        }

        capability_offers.push(CapabilityOffer { capability, offer_id });
    }

    Ok(ExecuteRequest { app_name, arguments, mode, source_offer, sink_offer, cwd_offer, capability_offers })
}

fn decode_metadata_response(reader: &mut PayloadReader<'_>) -> Result<AppMetadataResponse, Error> {
    let status = reader.read_u32()?;
    let input = decode_io_mode(reader.read_u8()?)?;
    let modes = decode_io_modes(reader.read_u8()?);
    let default_mode = decode_io_mode(reader.read_u8()?)?;
    let _flags = reader.read_u8()?;
    let app_id = reader.read_string(MAX_APP_NAME, "application ID")?;
    let display_name = reader.read_string(MAX_APP_NAME, "application display name")?;
    Ok(AppMetadataResponse { status, input, modes, default_mode, app_id, display_name })
}

fn decode_execute_response(reader: &mut PayloadReader<'_>) -> Result<ExecuteResponse, Error> {
    let status = reader.read_u32()?;
    let raw_offer = reader.read_u64()?;
    let process_offer = if raw_offer == 0 {
        None
    } else {
        Some(usize::try_from(raw_offer).map_err(|_| Error::invalid_argument("process offer is out of range".into()))?)
    };
    let message = reader.read_string(4096, "execution response message")?;
    Ok(ExecuteResponse { status, process_offer, message })
}

pub fn decode_io_modes_strings(values: &[String]) -> Result<AppIoModes, Error> {
    let mut modes = AppIoModes::new();
    for value in values {
        let mode = decode_io_mode_string(value.as_str())?;
        if mode == AppIoMode::Any {
            return Err(Error::invalid_argument("application mode set cannot contain any".into()));
        }
        modes = modes | AppIoModes::from_mode(mode);
    }
    if modes == AppIoModes::new() {
        return Err(Error::invalid_argument("application must support at least one mode".into()));
    }
    Ok(modes)
}

pub fn recv_hesper_request(socket: &Socket) -> Result<HesperRequest, Error> {
    let frame = socket.recv_frame()?;
    let mut reader = PayloadReader::new(&frame.payload);
    let request_id = read_hesper_header(&mut reader)?;

    match frame.header.packet_type {
        HESPER_APP_METADATA_REQUEST => {
            let request = decode_metadata_request(&mut reader)?;
            reader.finish()?;
            Ok(HesperRequest::AppMetadata { request_id, request })
        }

        HESPER_EXECUTE_REQUEST => {
            let request = decode_execute_request(&mut reader)?;
            reader.finish()?;
            Ok(HesperRequest::Execute { request_id, request })
        }

        _ => Err(Error::invalid_argument("unknown Hesper request type".into())),
    }
}

pub fn recv_hesper_response(socket: &Socket) -> Result<HesperResponse, Error> {
    let frame = socket.recv_frame()?;
    let mut reader = PayloadReader::new(&frame.payload);
    let request_id = read_hesper_header(&mut reader)?;

    match frame.header.packet_type {
        HESPER_APP_METADATA_RESPONSE => {
            let response = decode_metadata_response(&mut reader)?;
            reader.finish()?;
            Ok(HesperResponse::AppMetadata { request_id, response })
        }

        HESPER_EXECUTE_RESPONSE => {
            let response = decode_execute_response(&mut reader)?;
            reader.finish()?;
            Ok(HesperResponse::Execute { request_id, response })
        }

        _ => Err(Error::invalid_argument("unknown Hesper response type".into())),
    }
}

// ---------- OUTBOUND ------------//

fn write_hesper_header(output: &mut Vec<u8>, request_id: u32) {
    write_u16(output, 0); // flags
    write_u32(output, request_id);
}

fn write_capability_id(output: &mut Vec<u8>, capability: CapabilityID) -> Result<(), Error> {
    if capability.0 == 0 {
        return Err(Error::invalid_argument("capability ID cannot be zero".into()));
    }

    let raw = u64::try_from(capability.0).map_err(|_| Error::invalid_argument("capability ID is out of range".into()))?;

    write_u64(output, raw);
    Ok(())
}

pub fn send_app_metadata_request(socket: &Socket, request_id: u32, app_name: &str) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_hesper_header(&mut payload, request_id);
    write_string(&mut payload, app_name, MAX_APP_NAME, "application name")?;
    socket.send_frame(HESPER_APP_METADATA_REQUEST, &payload)
}

pub fn send_execute_request(
    socket: &Socket, request_id: u32, app_name: &str, arguments: &[String], mode: AppIoMode,
    source_offer: PortalOfferId, sink_offer: PortalOfferId,
    cwd_offer: PortalOfferId, capability_offers: &[CapabilityOffer],
) -> Result<(), Error> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(Error::invalid_argument("too many application arguments".into()));
    }
    let mut payload = Vec::new();
    write_hesper_header(&mut payload, request_id);
    write_string(&mut payload, app_name, MAX_APP_NAME, "application name")?;
    write_u32(&mut payload, u32::try_from(arguments.len()).map_err(|_| Error::invalid_argument("too many application arguments".into()))?);

    for argument in arguments {
        write_string(&mut payload, argument, MAX_ARGUMENT_LENGTH, "application argument")?;
    }

    write_u8(&mut payload, mode as u8);
    write_u8(&mut payload, 0);
    write_u16(&mut payload, 0);

    write_offer_id(&mut payload, source_offer, "source offer")?;
    write_offer_id(&mut payload, sink_offer, "sink offer")?;
    write_offer_id(&mut payload, cwd_offer, "cwd offer")?;
    if capability_offers.len() > MAX_CAPABILITY_OFFERS {
        return Err(Error::invalid_argument("too many capability offers".into()));
    }

    write_u32(
        &mut payload,
        u32::try_from(capability_offers.len()).map_err(|_| Error::invalid_argument("too many capability offers".into()))?,
    );

    for offer in capability_offers {
        write_capability_id(&mut payload, offer.capability)?;

        write_offer_id(&mut payload, offer.offer_id, "capability offer")?;
    }
    socket.send_frame(HESPER_EXECUTE_REQUEST, &payload)
}

pub fn send_app_metadata_response(
    socket: &Socket, request_id: u32, status: u32, 
    input: AppIoMode, modes: AppIoModes, default_mode: AppIoMode,
    app_id: &str, display_name: &str,
) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_hesper_header(&mut payload, request_id);
    write_u32(&mut payload, status);
    write_u8(&mut payload, input as u8);
    write_u8(&mut payload, encode_io_modes(modes));
    write_u8(&mut payload, default_mode as u8);
    write_u8(&mut payload, 0);
    write_string(&mut payload, app_id, MAX_APP_NAME, "application ID")?;
    write_string(&mut payload, display_name, MAX_APP_NAME, "application display name")?;
    socket.send_frame(HESPER_APP_METADATA_RESPONSE, &payload)
}

pub fn send_execute_response(
    socket: &Socket, request_id: u32, status: u32, process_offer: Option<PortalOfferId>, message: &str,
) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_hesper_header(&mut payload, request_id);
    write_u32(&mut payload, status);
    match process_offer {
        Some(offer_id) => write_offer_id(&mut payload, offer_id, "process offer")?,
        None => write_u64(&mut payload, 0),
    }
    write_string(&mut payload, message, 4096, "execution response message")?;
    socket.send_frame(HESPER_EXECUTE_RESPONSE, &payload)
}

pub struct Launcher {
    socket: Socket,
    next_id: u32,
}

impl Launcher {
    pub fn connect() -> Result<Self, Error> {
        let portal_handle = resolve(&Path::new("/System/Services/Launcher"), AccessRights::READ)?;
        let portal = Broker::from_handle(portal_handle);
        let socket_handle = portal.request(CAP_LAUNCHER_CONNECT, AccessRights::READ | AccessRights::WRITE)?;

        Ok(Self { socket: Socket::from_handle(socket_handle), next_id: 1 })
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        if self.next_id == 0 {
            self.next_id = 1;
        }
        id
    }

    pub fn metadata(&mut self, app_name: &str) -> Result<AppMetadataResponse, Error> {
        let request_id = self.next_id();

        send_app_metadata_request(&self.socket, request_id, app_name)?;

        match recv_hesper_response(&self.socket)? {
            HesperResponse::AppMetadata { request_id: response_id, response } => {
                if response_id != request_id {
                    return Err(Error::invalid_argument("Hesper response ID mismatch".into()));
                }
                Ok(response)
            }

            HesperResponse::Execute { .. } => Err(Error::invalid_argument("Hesper returned the wrong response type".into())),
        }
    }

    pub fn execute(
        &mut self, app_name: &str, arguments: &[String], mode: AppIoMode,
        source_offer: PortalOfferId, sink_offer: PortalOfferId, cwd_offer: PortalOfferId,
        capability_offers: &[CapabilityOffer],
    ) -> Result<ExecuteResponse, Error> {
        let request_id = self.next_id();

        send_execute_request(&self.socket, request_id, app_name, arguments, mode, 
            source_offer, sink_offer, cwd_offer, capability_offers)?;

        match recv_hesper_response(&self.socket)? {
            HesperResponse::Execute { request_id: response_id, response } => {
                if response_id != request_id {
                    return Err(Error::invalid_argument("Hesper response ID mismatch".into()));
                }
                Ok(response)
            }
            HesperResponse::AppMetadata { .. } => Err(Error::invalid_argument("Hesper returned the wrong response type".into())),
        }
    }

    pub fn offer(&self, handle: HandleID, max_rights: AccessRights) -> Result<PortalOfferId, Error> {
        offer_handle(self.socket.handle(), handle, max_rights)
    }

    pub fn accept(&self, offer_id: PortalOfferId, requested_rights: AccessRights) -> Result<HandleID, Error> {
        accept_handle(self.socket.handle(), offer_id, requested_rights)
    }

    pub fn revoke(&self, offer_id: PortalOfferId) -> Result<(), Error> { revoke_offer(self.socket.handle(), offer_id) }

    pub fn launch(
        &mut self, app_name: &str, arguments: &[String], mode: AppIoMode,
        source: HandleID, sink: HandleID, cwd: HandleID,
    ) -> Result<Process, Error> {
        let mut pending_offers = Vec::new();

        let response_result = (|| {
            let source_offer = self.offer(source, AccessRights::READ)?;
            pending_offers.push(source_offer);

            let sink_offer = self.offer(sink, AccessRights::WRITE)?;
            pending_offers.push(sink_offer);

            let cwd_offer = self.offer(cwd, AccessRights::TRAVERSE | AccessRights::LIST)?;
            pending_offers.push(cwd_offer);

            let environment_capabilities = env::capabilities();

            if environment_capabilities.len() > MAX_CAPABILITY_OFFERS {
                return Err(Error::invalid_argument("too many environment capabilities".into()));
            }

            let mut capability_offers = Vec::with_capacity(environment_capabilities.len());

            for grant in environment_capabilities {
                if capability_offers.iter().any(|offer: &CapabilityOffer| offer.capability == grant.capability) {
                    return Err(Error::invalid_argument("duplicate environment capability".into()));
                }

                let offer_id = self.offer(grant.id, grant.rights)?;

                pending_offers.push(offer_id);

                capability_offers.push(CapabilityOffer { capability: grant.capability, offer_id });
            }

            self.execute(app_name, arguments, mode, source_offer, sink_offer, cwd_offer, &capability_offers)
        })();

        for offer_id in pending_offers {
            let _ = self.revoke(offer_id);
        }

        let response = response_result?;

        if response.status != HESPER_STATUS_OK {
            return Err(match response.status {
                HESPER_STATUS_NOT_FOUND => Error::not_found("application was not found".into()),

                HESPER_STATUS_INVALID_REQUEST => Error::invalid_argument(response.message),

                HESPER_STATUS_LAUNCH_FAILED => Error::unknown(response.message),

                _ => Error::unknown("Hesper returned an unknown status".into()),
            });
        }

        let process_offer = response
            .process_offer
            .ok_or_else(|| Error::invalid_argument("successful launch response omitted process capability".into()))?;

        let process_handle = self.accept(process_offer, AccessRights::READ)?;

        Ok(Process::from_handle(process_handle))
    }
}

pub fn decode_io_modes(value: u8) -> AppIoModes {
    AppIoModes(value)
}

pub fn encode_io_modes(value: AppIoModes) -> u8 {
    value.0
}
