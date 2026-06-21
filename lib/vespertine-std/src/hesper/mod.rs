extern crate alloc;

use alloc::string::{
    String,
    ToString,
};
use alloc::vec::Vec;

use vespertine_abi::app::hesper::*;

use crate::Error;
use crate::socket::Socket;

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
    pub output: AppIoMode,
    pub app_id: String,
    pub display_name: String,
}

#[derive(Debug)]
pub struct ExecuteRequest {
    pub app_name: String,
    pub arguments: Vec<String>,
}

#[derive(Debug)]
pub struct ExecuteResponse {
    pub status: u32,
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

struct PayloadReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PayloadReader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, offset: 0 } }

    fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self.offset.checked_add(length).ok_or_else(|| Error::invalid_argument("payload length overflow".into()))?;

        if end > self.bytes.len() {
            return Err(Error::invalid_argument("truncated packet payload".into()));
        }

        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<u8, Error> { Ok(self.take(1)?[0]) }

    fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, Error> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_string(&mut self, maximum: usize, description: &str) -> Result<String, Error> {
        let length = self.read_u32()? as usize;

        if length > maximum {
            return Err(Error::invalid_argument(alloc::format!("{} is too long", description)));
        }

        let bytes = self.take(length)?;
        let value = str::from_utf8(bytes).map_err(|_| Error::invalid_encoding(alloc::format!("{} is not UTF-8", description)))?;

        Ok(value.to_string())
    }

    fn finish(self) -> Result<(), Error> {
        if self.offset != self.bytes.len() {
            return Err(Error::invalid_argument("packet contains trailing data".into()));
        }

        Ok(())
    }
}

fn write_u8(output: &mut Vec<u8>, value: u8) { output.push(value); }

fn write_u16(output: &mut Vec<u8>, value: u16) { output.extend_from_slice(&value.to_le_bytes()); }

fn write_u32(output: &mut Vec<u8>, value: u32) { output.extend_from_slice(&value.to_le_bytes()); }

fn write_string(output: &mut Vec<u8>, value: &str, maximum: usize, description: &str) -> Result<(), Error> {
    if value.len() > maximum {
        return Err(Error::invalid_argument(alloc::format!("{} is too long", description)));
    }

    let length = u32::try_from(value.len()).map_err(|_| Error::invalid_argument(alloc::format!("{} is too long", description)))?;

    write_u32(output, length);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

pub fn decode_io_mode(value: u8) -> Result<AppIoMode, Error> {
    match value {
        0 => Ok(AppIoMode::Any),
        1 => Ok(AppIoMode::Text),
        2 => Ok(AppIoMode::Typed),
        3 => Ok(AppIoMode::Direct),
        _ => Err(Error::invalid_argument("invalid app I/O mode".into())),
    }
}

pub fn decode_io_mode_string(value: &str) -> Result<AppIoMode, Error> {
    match value {
        "any" => Ok(AppIoMode::Any),
        "text" => Ok(AppIoMode::Text),
        "typed" => Ok(AppIoMode::Typed),
        "direct" => Ok(AppIoMode::Direct),
        _ => Err(Error::invalid_argument("invalid app I/O mode".into())),
    }
}

fn decode_metadata_request(payload: &[u8]) -> Result<AppMetadataRequest, Error> {
    let mut reader = PayloadReader::new(payload);
    let app_name = reader.read_string(MAX_APP_NAME, "application name")?;

    reader.finish()?;
    Ok(AppMetadataRequest { app_name })
}

fn decode_execute_request(payload: &[u8]) -> Result<ExecuteRequest, Error> {
    let mut reader = PayloadReader::new(payload);

    let app_name = reader.read_string(MAX_APP_NAME, "application name")?;

    let argument_count = reader.read_u32()? as usize;
    if argument_count > MAX_ARGUMENTS {
        return Err(Error::invalid_argument("too many application arguments".into()));
    }

    let mut arguments = Vec::new();

    for _ in 0..argument_count {
        arguments.push(reader.read_string(MAX_ARGUMENT_LENGTH, "application argument")?);
    }

    reader.finish()?;
    Ok(ExecuteRequest { app_name, arguments })
}

fn decode_metadata_response(payload: &[u8]) -> Result<AppMetadataResponse, Error> {
    let mut reader = PayloadReader::new(payload);

    let status = reader.read_u32()?;
    let input = decode_io_mode(reader.read_u8()?)?;
    let output = decode_io_mode(reader.read_u8()?)?;

    // reserved response flags.
    let _flags = reader.read_u16()?;

    let app_id = reader.read_string(MAX_APP_NAME, "application ID")?;

    let display_name = reader.read_string(MAX_APP_NAME, "application display name")?;

    reader.finish()?;

    Ok(AppMetadataResponse { status, input, output, app_id, display_name })
}

fn decode_execute_response(payload: &[u8]) -> Result<ExecuteResponse, Error> {
    let mut reader = PayloadReader::new(payload);

    let status = reader.read_u32()?;
    let message = reader.read_string(4096, "execution response message")?;

    reader.finish()?;
    Ok(ExecuteResponse { status, message })
}

pub fn recv_hesper_request(socket: &Socket) -> Result<HesperRequest, Error> {
    let frame = socket.recv_frame()?;
    let request_id = frame.header.reserved;

    match frame.header.packet_type {
        HESPER_APP_METADATA_REQUEST => {
            let request = decode_metadata_request(&frame.payload)?;

            Ok(HesperRequest::AppMetadata { request_id, request })
        }

        HESPER_EXECUTE_REQUEST => {
            let request = decode_execute_request(&frame.payload)?;

            Ok(HesperRequest::Execute { request_id, request })
        }

        _ => Err(Error::invalid_argument("unknown Hesper request type".into())),
    }
}

pub fn recv_hesper_response(socket: &Socket) -> Result<HesperResponse, Error> {
    let frame = socket.recv_frame()?;
    let request_id = frame.header.reserved;

    match frame.header.packet_type {
        HESPER_APP_METADATA_RESPONSE => {
            let response = decode_metadata_response(&frame.payload)?;

            Ok(HesperResponse::AppMetadata { request_id, response })
        }

        HESPER_EXECUTE_RESPONSE => {
            let response = decode_execute_response(&frame.payload)?;

            Ok(HesperResponse::Execute { request_id, response })
        }

        _ => Err(Error::invalid_argument("unknown Hesper response type".into())),
    }
}

pub fn send_app_metadata_request(socket: &Socket, request_id: u32, app_name: &str) -> Result<(), Error> {
    let mut payload = Vec::new();

    write_string(&mut payload, app_name, MAX_APP_NAME, "application name")?;

    socket.send_frame(HESPER_APP_METADATA_REQUEST, request_id, &payload)
}

pub fn send_execute_request(socket: &Socket, request_id: u32, app_name: &str, arguments: &[String]) -> Result<(), Error> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(Error::invalid_argument("too many application arguments".into()));
    }

    let mut payload = Vec::new();

    write_string(&mut payload, app_name, MAX_APP_NAME, "application name")?;

    write_u32(&mut payload, u32::try_from(arguments.len()).map_err(|_| Error::invalid_argument("too many application arguments".into()))?);

    for argument in arguments {
        write_string(&mut payload, argument, MAX_ARGUMENT_LENGTH, "application argument")?;
    }

    socket.send_frame(HESPER_EXECUTE_REQUEST, request_id, &payload)
}

pub fn send_app_metadata_response(
    socket: &Socket, request_id: u32, status: u32, input: AppIoMode, output: AppIoMode, app_id: &str, display_name: &str,
) -> Result<(), Error> {
    let mut payload = Vec::new();

    write_u32(&mut payload, status);
    write_u8(&mut payload, input as u8);
    write_u8(&mut payload, output as u8);
    write_u16(&mut payload, 0);

    write_string(&mut payload, app_id, MAX_APP_NAME, "application ID")?;

    write_string(&mut payload, display_name, MAX_APP_NAME, "application display name")?;

    socket.send_frame(HESPER_APP_METADATA_RESPONSE, request_id, &payload)
}

pub fn send_execute_response(socket: &Socket, request_id: u32, status: u32, message: &str) -> Result<(), Error> {
    let mut payload = Vec::new();

    write_u32(&mut payload, status);
    write_string(&mut payload, message, 4096, "execution response message")?;

    socket.send_frame(HESPER_EXECUTE_RESPONSE, request_id, &payload)
}
