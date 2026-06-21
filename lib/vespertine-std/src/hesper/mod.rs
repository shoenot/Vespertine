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
    let mut arguments = Vec::new();
    for _ in 0..argument_count {
        arguments.push(reader.read_string(MAX_ARGUMENT_LENGTH, "application argument")?);
    }
    Ok(ExecuteRequest { app_name, arguments })
}

fn decode_metadata_response(reader: &mut PayloadReader<'_>) -> Result<AppMetadataResponse, Error> {
    let status = reader.read_u32()?;
    let input = decode_io_mode(reader.read_u8()?)?;
    let output = decode_io_mode(reader.read_u8()?)?;
    let _flags = reader.read_u16()?;
    let app_id = reader.read_string(MAX_APP_NAME, "application ID")?;
    let display_name = reader.read_string(MAX_APP_NAME, "application display name")?;
    Ok(AppMetadataResponse { status, input, output, app_id, display_name })
}

fn decode_execute_response(reader: &mut PayloadReader<'_>) -> Result<ExecuteResponse, Error> {
    let status = reader.read_u32()?;
    let message = reader.read_string(4096, "execution response message")?;
    Ok(ExecuteResponse { status, message })
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

            Ok(HesperResponse::AppMetadata { request_id, response })
        }

        HESPER_EXECUTE_RESPONSE => {
            let response = decode_execute_response(&mut reader)?;

            Ok(HesperResponse::Execute { request_id, response })
        }

        _ => Err(Error::invalid_argument("unknown Hesper response type".into())),
    }
}

// ---------- OUTBOUND ------------//

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

fn write_hesper_header(output: &mut Vec<u8>, request_id: u32) {
    write_u16(output, 0); // flags
    write_u32(output, request_id);
}


pub fn send_app_metadata_request(socket: &Socket, request_id: u32, app_name: &str) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_hesper_header(&mut payload, request_id);
    write_string(&mut payload, app_name, MAX_APP_NAME, "application name")?;
    socket.send_frame(HESPER_APP_METADATA_REQUEST, &payload)
}

pub fn send_execute_request(socket: &Socket, request_id: u32, app_name: &str, arguments: &[String]) -> Result<(), Error> {
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
    socket.send_frame(HESPER_EXECUTE_REQUEST, &payload)
}

pub fn send_app_metadata_response(
    socket: &Socket, request_id: u32, status: u32, input: AppIoMode, output: AppIoMode, app_id: &str, display_name: &str,
) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_hesper_header(&mut payload, request_id);
    write_u32(&mut payload, status);
    write_u8(&mut payload, input as u8);
    write_u8(&mut payload, output as u8);
    write_u16(&mut payload, 0);
    write_string(&mut payload, app_id, MAX_APP_NAME, "application ID")?;
    write_string(&mut payload, display_name, MAX_APP_NAME, "application display name")?;
    socket.send_frame(HESPER_APP_METADATA_RESPONSE, &payload)
}

pub fn send_execute_response(socket: &Socket, request_id: u32, status: u32, message: &str) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_hesper_header(&mut payload, request_id);
    write_u32(&mut payload, status);
    write_string(&mut payload, message, 4096, "execution response message")?;
    socket.send_frame(HESPER_EXECUTE_RESPONSE, &payload)
}
