extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use vespertine_abi::app::hesper::{
    AppIoMode,
    AppIoModes,
};
use vespertine_abi::app::vreg::*;
use vespertine_abi::tag::CAP_VREG_CONNECT;
use vespertine_abi::AccessRights;
use vespertine_abi::typed::{DATETIME_EMPTY, DateTimeValue};

use crate::broker::Broker;
use crate::fs::{
    Path,
    resolve,
};
use crate::hesper::{
    decode_io_mode,
    decode_io_modes,
    encode_io_modes,
};
use crate::payload::{PayloadReader, write_string, write_u8, write_u16, write_u32};
use crate::socket::Socket;
use crate::typed::{DateTimeValueExt, convert_iso_datetime};
use crate::{
    Error,
    ErrorKind,
};

const MAX_APP_NAME: usize = 256;
const MAX_APP_ID: usize = 256;
const MAX_DISPLAY_NAME: usize = 256;
const MAX_BUNDLE_PATH: usize = 4096;
const MAX_ENTRYPOINT: usize = 256;
const MAX_BINARY_NAME: usize = 256;
const MAX_TIMESTAMP: usize = 64;

#[derive(Debug, Clone)]
pub struct ResolvedApplication {
    pub command: String,
    pub app_id: String,
    pub bundle: String,
    pub entrypoint: String,
    pub binary: String,
    pub input: AppIoMode,
    pub modes: AppIoModes,
    pub default_mode: AppIoMode,
    pub display_name: String,

    pub installed_ts: DateTimeValue,
    pub updated_ts: DateTimeValue,
}

#[derive(Debug)]
pub enum VRegistryRequest {
    Resolve { request_id: u32, name: String },
    List { request_id: u32 },
    Reload { request_id: u32 },
}

#[derive(Debug)]
pub enum VRegistryResponse {
    Resolve {
        request_id: u32,
        status: u32,
        app: Option<ResolvedApplication>,
    },

    ListEntry {
        request_id: u32,
        app: ResolvedApplication,
    },

    ListEnd {
        request_id: u32,
        status: u32,
    },
    Reload {
        request_id: u32,
        status: u32,
    }
}

fn read_vreg_header(reader: &mut PayloadReader<'_>) -> Result<u32, Error> {
    let _flags = reader.read_u16()?;
    let request_id = reader.read_u32()?;
    Ok(request_id)
}

fn write_vreg_header(output: &mut Vec<u8>, request_id: u32) {
    write_u16(output, 0);
    write_u32(output, request_id);
}

fn read_application(reader: &mut PayloadReader<'_>) -> Result<ResolvedApplication, Error> {
    let command = reader.read_string(MAX_APP_NAME, "application command")?;
    let app_id = reader.read_string(MAX_APP_ID, "application ID")?;
    let bundle = reader.read_string(MAX_BUNDLE_PATH, "application bundle path")?;
    let entrypoint = reader.read_string(MAX_ENTRYPOINT, "application entrypoint")?;
    let binary = reader.read_string(MAX_BINARY_NAME, "application binary")?;
    let input = decode_io_mode(reader.read_u8()?)?;
    let modes = decode_io_modes(reader.read_u8()?);
    let default_mode = decode_io_mode(reader.read_u8()?)?;
    let _flags = reader.read_u8()?;
    let display_name = reader.read_string(MAX_DISPLAY_NAME, "application display name")?;
    let installed_ts = convert_iso_datetime(reader.read_string(MAX_TIMESTAMP, "application install timestamp")?);
    let updated_ts = convert_iso_datetime(reader.read_string(MAX_TIMESTAMP, "application update timestamp")?);

    Ok(ResolvedApplication {
        command, app_id, bundle, entrypoint, binary,
        input, modes, default_mode, display_name,
        installed_ts, updated_ts
    })
}

fn write_application(output: &mut Vec<u8>, app: &ResolvedApplication) -> Result<(), Error> {
    write_string(output, &app.command, MAX_APP_NAME, "application command")?;
    write_string(output, &app.app_id, MAX_APP_ID, "application ID")?;
    write_string(output, &app.bundle, MAX_BUNDLE_PATH, "application bundle path")?;
    write_string(output, &app.entrypoint, MAX_ENTRYPOINT, "application entrypoint")?;
    write_string(output, &app.binary, MAX_BINARY_NAME, "application binary")?;
    write_u8(output, app.input as u8);
    write_u8(output, encode_io_modes(app.modes));
    write_u8(output, app.default_mode as u8);
    write_u8(output, 0);
    write_string(output, &app.display_name, MAX_DISPLAY_NAME, "application display name")?;
    write_string(output, &app.installed_ts.as_iso_string(), MAX_TIMESTAMP, "application install timestamp")?;
    write_string(output, &app.updated_ts.as_iso_string(), MAX_TIMESTAMP, "application update timestamp")?;
    Ok(())
}

pub fn send_resolve_request(socket: &Socket, request_id: u32, name: &str) -> Result<(), Error> {
    let mut payload = Vec::new();

    write_vreg_header(&mut payload, request_id);
    write_string(&mut payload, name, MAX_APP_NAME, "application name")?;

    socket.send_frame(VREG_RESOLVE_REQUEST, &payload)
}

pub fn send_list_request(socket: &Socket, request_id: u32) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_vreg_header(&mut payload, request_id);
    socket.send_frame(VREG_LIST_REQUEST, &payload)
}

pub fn send_reload_request(socket: &Socket, request_id: u32) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_vreg_header(&mut payload, request_id);
    socket.send_frame(VREG_RELOAD_REQUEST, &payload)
}

pub fn send_resolve_response(socket: &Socket, request_id: u32, status: u32, app:
Option<&ResolvedApplication>) -> Result<(), Error> {
    let mut payload = Vec::new();

    write_vreg_header(&mut payload, request_id);
    write_u32(&mut payload, status);

    if status == VREG_STATUS_OK {
        let app = app.ok_or_else(|| Error::invalid_argument("successful vreg response omitted application".into()))?;
        write_application(&mut payload, app)?;
    }

    socket.send_frame(VREG_RESOLVE_RESPONSE, &payload)
}

pub fn send_list_entry(socket: &Socket, request_id: u32, app: &ResolvedApplication) -> Result<(),
Error> {
    let mut payload = Vec::new();

    write_vreg_header(&mut payload, request_id);
    write_application(&mut payload, app)?;

    socket.send_frame(VREG_LIST_ENTRY, &payload)
}

pub fn send_list_end(socket: &Socket, request_id: u32, status: u32) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_vreg_header(&mut payload, request_id);
    write_u32(&mut payload, status);
    socket.send_frame(VREG_LIST_END, &payload)
}

pub fn send_reload_response(socket: &Socket, request_id: u32, status: u32) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_vreg_header(&mut payload, request_id);
    write_u32(&mut payload, status);
    socket.send_frame(VREG_RELOAD_RESPONSE, &payload)
}

pub fn recv_vreg_request(socket: &Socket) -> Result<VRegistryRequest, Error> {
    let frame = socket.recv_frame()?;
    let mut reader = PayloadReader::new(&frame.payload);
    let request_id = read_vreg_header(&mut reader)?;

    match frame.header.packet_type {
        VREG_RESOLVE_REQUEST => {
            let name = reader.read_string(MAX_APP_NAME, "application name")?;
            reader.finish()?;
            Ok(VRegistryRequest::Resolve { request_id, name })
        },
        VREG_LIST_REQUEST => {
            reader.finish()?;
            Ok(VRegistryRequest::List { request_id })
        },
        VREG_RELOAD_REQUEST => {
            reader.finish()?;
            Ok(VRegistryRequest::Reload { request_id })
        },
        _ => Err(Error::invalid_argument("unknown vreg request type".into())),
    }
}

pub fn recv_vreg_response(socket: &Socket) -> Result<VRegistryResponse, Error> {
    let frame = socket.recv_frame()?;
    let mut reader = PayloadReader::new(&frame.payload);
    let request_id = read_vreg_header(&mut reader)?;

    match frame.header.packet_type {
        VREG_RESOLVE_RESPONSE => {
            let status = reader.read_u32()?;
            let app = if status == VREG_STATUS_OK { Some(read_application(&mut reader)?) } else { None };
            reader.finish()?;
            Ok(VRegistryResponse::Resolve { request_id, status, app })
        },
        VREG_LIST_ENTRY => {
            let app = read_application(&mut reader)?;
            reader.finish()?;
            Ok(VRegistryResponse::ListEntry { request_id, app })
        },
        VREG_LIST_END => {
            let status = reader.read_u32()?;
            reader.finish()?;
            Ok(VRegistryResponse::ListEnd { request_id, status })
        },
        VREG_RELOAD_RESPONSE => {
            let status = reader.read_u32()?;
            reader.finish()?;
            Ok(VRegistryResponse::Reload { request_id, status })
        },
        _ => Err(Error::invalid_argument("unknown vreg response type".into())),
    }
}

pub struct VRegistryClient {
    socket: Socket,
    next_id: u32,
}

impl VRegistryClient {
    pub fn connect() -> Result<Self, Error> {
        let portal_handle = resolve(&Path::new("/System/Services/VRegistry"), AccessRights::READ)?;
        let portal = Broker::from_handle(portal_handle);
        let socket_handle = portal.request(CAP_VREG_CONNECT, AccessRights::READ | AccessRights::WRITE)?;

        Ok(Self { socket: Socket::from_handle(socket_handle), next_id: 1 })
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        if self.next_id == 0 { self.next_id = 1; }
        id
    }

    pub fn resolve(&mut self, name: &str) -> Result<ResolvedApplication, Error> {
        let request_id = self.next_id();

        send_resolve_request(&self.socket, request_id, name)?;

        match recv_vreg_response(&self.socket)? {
            VRegistryResponse::Resolve { request_id: response_id, status, app } => {
                if response_id != request_id { return Err(Error::invalid_argument("vreg response ID mismatch".into())); }
                if status != VREG_STATUS_OK {
                    return Err(match status {
                        VREG_STATUS_NOT_FOUND => Error::not_found("application was not found".into()),
                        VREG_STATUS_INVALID_REQUEST => Error::invalid_argument("vreg rejected resolve request".into()),
                        _ => Error::unknown("vreg returned an unknown resolve status".into()),
                    });
                }
                app.ok_or_else(|| Error::invalid_argument("successful vreg response omitted application".into()))
            },
            _ => Err(Error::invalid_argument("vreg returned the wrong response type".into())),
        }
    }

    pub fn list(&mut self) -> Result<Vec<ResolvedApplication>, Error> {
        let request_id = self.next_id();

        send_list_request(&self.socket, request_id)?;

        let mut apps = Vec::new();

        loop {
            match recv_vreg_response(&self.socket)? {
                VRegistryResponse::ListEntry { request_id: response_id, app } => {
                    if response_id != request_id { return Err(Error::invalid_argument("vreg response ID mismatch".into())); }
                    apps.push(app);
                },
                VRegistryResponse::ListEnd { request_id: response_id, status } => {
                    if response_id != request_id { return Err(Error::invalid_argument("vreg response ID mismatch".into())); }
                    if status == VREG_STATUS_OK { return Ok(apps); }
                    return Err(match status {
                        VREG_STATUS_INVALID_REQUEST => Error::invalid_argument("vreg rejected list request".into()),
                        _ => Error::unknown("vreg returned an unknown list status".into()),
                    });
                },
                VRegistryResponse::Resolve { .. } | VRegistryResponse::Reload { .. } => {
                    return Err(Error::invalid_argument("vreg returned the wrong response type".into()));
                },
            }
        }
    }

    pub fn reload(&mut self) -> Result<(), Error> {
        let request_id = self.next_id();
        
        send_reload_request(&self.socket, request_id)?;

        match recv_vreg_response(&self.socket)? {
            VRegistryResponse::Reload { request_id: response_id, status } => {
                if response_id != request_id { return Err(Error::invalid_argument("vreg response ID mismatch".into())); }
                if status == VREG_STATUS_OK { return Ok(()); }

                Err(match status {
                    VREG_STATUS_INVALID_REQUEST => Error::invalid_argument("vreg rejected reload request".into()),
                    VREG_STATUS_INTERNAL_ERROR => Error::unknown("vreg reload failed".into()),
                    _ => Error::unknown("vreg returned an unknown reload status".into()),
                })
            },
            _ => return Err(Error::invalid_argument("vreg returned the wrong response type".into())),
        }
    }
}

pub fn status_from_error(error: &Error) -> u32 {
    match error.kind {
        ErrorKind::NotFound => VREG_STATUS_NOT_FOUND,
        ErrorKind::InvalidArgument | ErrorKind::InvalidEncoding | ErrorKind::NameTooLong => VREG_STATUS_INVALID_REQUEST,
        _ => VREG_STATUS_INTERNAL_ERROR,
    }
}
