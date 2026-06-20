extern crate alloc;

use alloc::string::String;

use vespertine_abi::app::hesper::*;

use crate::{
    Error, Write,
    socket::{DecodePacketPayload, PacketPayload, Socket},
};

pub struct AppMetadataRequest<'a> {
    pub app_name: &'a str,
}

pub struct OwnedAppMetadataRequest {
    pub app_name: String,
}

pub struct AppMetadataResponse<'a> {
    pub status: u32,
    pub input: AppIoMode,
    pub output: AppIoMode,
    pub app_id: &'a str,
    pub display_name: &'a str,
}

pub struct OwnedAppMetadataResponse {
    pub status: u32,
    pub input: AppIoMode,
    pub output: AppIoMode,
    pub app_id: String,
    pub display_name: String,
}

impl PacketPayload for AppMetadataRequest<'_> {
    fn payload_len(&self) -> usize {
        size_of::<AppMetadataRequestHeader>() + self.app_name.len()
    }

    fn write_payload(&self, socket: &Socket) -> Result<(), Error> {
        let header = AppMetadataRequestHeader {
            app_name_len: self.app_name.len() as u32,
        };

        header.write_payload(socket)?;
        socket.write_all(self.app_name.as_bytes())
    }
}

impl DecodePacketPayload for OwnedAppMetadataRequest {
    fn decode_packet_payload(payload: &[u8]) -> Result<Self, Error> {
        let header = read_plain::<AppMetadataRequestHeader>(payload)?;
        let offset = size_of::<AppMetadataRequestHeader>();

        let app_len = header.app_name_len as usize;
        if payload.len() != offset + app_len {
            return Err(Error::invalid_argument("bad metadata request length".into()));
        }

        let app_name = str::from_utf8(&payload[offset..offset + app_len])
            .map_err(|_| Error::invalid_argument("invalid app name utf8".into()))?
            .into();

        Ok(Self { app_name })
    }
}


fn read_plain<T: Copy>(payload: &[u8]) -> Result<T, Error> {
    if payload.len() < size_of::<T>() {
        return Err(Error::invalid_argument("short packet".into()));
    }

    let mut value = unsafe { core::mem::zeroed::<T>() };
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(
            &mut value as *mut T as *mut u8,
            size_of::<T>(),
        )
    };

    bytes.copy_from_slice(&payload[..size_of::<T>()]);
    Ok(value)
}

pub fn send_app_metadata_request(socket: &Socket, app_name: &str) -> Result<(), Error> {
    socket.send_packet(
        HESPER_APP_METADATA_REQUEST,
        &AppMetadataRequest { app_name },
    )
}

pub fn recv_app_metadata_request(socket: &Socket) -> Result<OwnedAppMetadataRequest, Error> {
    let (header, req) = socket.recv_packet::<OwnedAppMetadataRequest>()?;

    if header.packet_type != HESPER_APP_METADATA_REQUEST {
        return Err(Error::invalid_argument("unexpected packet type".into()));
    }

    Ok(req)
}


fn decode_io_mode(value: u8) -> Result<AppIoMode, Error> {
    match value {
        0 => Ok(AppIoMode::Any),
        1 => Ok(AppIoMode::Text),
        2 => Ok(AppIoMode::Typed),
        3 => Ok(AppIoMode::Direct),
        _ => Err(Error::invalid_argument("invalid app io mode".into())),
    }
}

impl PacketPayload for AppMetadataResponse<'_> {
    fn payload_len(&self) -> usize {
        size_of::<AppMetadataResponseHeader>()
            + self.app_id.len()
            + self.display_name.len()
    }

    fn write_payload(&self, socket: &Socket) -> Result<(), Error> {
        let header = AppMetadataResponseHeader {
            status: self.status,
            input: self.input as u8,
            output: self.output as u8,
            flags: 0,
            app_id_len: self.app_id.len() as u32,
            display_name_len: self.display_name.len() as u32,
        };

        header.write_payload(socket)?;
        socket.write_all(self.app_id.as_bytes())?;
        socket.write_all(self.display_name.as_bytes())?;

        Ok(())
    }
}

impl DecodePacketPayload for OwnedAppMetadataResponse {
    fn decode_packet_payload(payload: &[u8]) -> Result<Self, Error> {
        let header = read_plain::<AppMetadataResponseHeader>(payload)?;
        let mut offset = size_of::<AppMetadataResponseHeader>();

        let app_id_len = header.app_id_len as usize;
        let display_name_len = header.display_name_len as usize;

        if payload.len() != offset + app_id_len + display_name_len {
            return Err(Error::invalid_argument("bad metadata response length".into()));
        }

        let app_id = str::from_utf8(&payload[offset..offset + app_id_len])
            .map_err(|_| Error::invalid_argument("invalid app id utf8".into()))?
            .into();

        offset += app_id_len;

        let display_name = str::from_utf8(&payload[offset..offset + display_name_len])
            .map_err(|_| Error::invalid_argument("invalid display name utf8".into()))?
            .into();

        Ok(Self {
            status: header.status,
            input: decode_io_mode(header.input)?,
            output: decode_io_mode(header.output)?,
            app_id,
            display_name,
        })
    }
}

pub fn send_app_metadata_response(
    socket: &Socket,
    status: u32,
    input: AppIoMode,
    output: AppIoMode,
    app_id: &str,
    display_name: &str,
) -> Result<(), Error> {
    socket.send_packet(
        HESPER_APP_METADATA_RESPONSE,
        &AppMetadataResponse {
            status,
            input,
            output,
            app_id,
            display_name,
        },
    )
}

pub fn recv_app_metadata_response(socket: &Socket) -> Result<OwnedAppMetadataResponse, Error> {
    let (header, response) = socket.recv_packet::<OwnedAppMetadataResponse>()?;

    if header.packet_type != HESPER_APP_METADATA_RESPONSE {
        return Err(Error::invalid_argument("unexpected packet type".into()));
    }

    Ok(response)
}
