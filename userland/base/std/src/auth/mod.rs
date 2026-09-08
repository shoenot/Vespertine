extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::read_unaligned;
use core::slice;

use vespertine_abi::app::auth::*;
use vespertine_abi::tag::CAP_AUTH_CONNECT;
use vespertine_abi::typed::UserValue;
use vespertine_abi::{
    AccessRights,
    UserID,
};

use crate::broker::Broker;
use crate::fs::{
    Path,
    resolve,
};
use crate::payload::{
    PayloadReader,
    write_string,
    write_u16,
    write_u32,
};
use crate::socket::Socket;
use crate::{
    Error,
    ErrorKind,
};

const MAX_USER_NAME: usize = 64;
const MAX_USER_DISPLAY_NAME: usize = 255;
const MAX_HOME_PATH: usize = 4096;
const MAX_ROLE: usize = 64;
const MAX_ROLES: usize = 32;

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub user: UserValue,
    pub home: String,
    pub roles: Vec<String>,
}

#[derive(Debug)]
pub enum AuthRequest {
    DefaultUser { request_id: u32 },
    LookupId { request_id: u32, user: UserID },
    LookupName { request_id: u32, name: String },
}

#[derive(Debug)]
pub enum AuthResponse {
    Account { request_id: u32, status: u32, account: Option<AccountInfo> },
}

fn read_auth_header(reader: &mut PayloadReader<'_>) -> Result<u32, Error> {
    let _flags = reader.read_u16()?;
    let request_id = reader.read_u32()?;
    Ok(request_id)
}

fn write_auth_header(output: &mut Vec<u8>, request_id: u32) {
    write_u16(output, 0);
    write_u32(output, request_id);
}

fn write_user_value(output: &mut Vec<u8>, user: &UserValue) {
    let bytes = unsafe { slice::from_raw_parts(user as *const _ as *const u8, size_of::<UserValue>()) };
    output.extend_from_slice(bytes);
}

fn read_user_value(reader: &mut PayloadReader<'_>) -> Result<UserValue, Error> {
    let bytes = reader.take(size_of::<UserValue>())?;
    Ok(unsafe { read_unaligned(bytes.as_ptr() as *const UserValue) })
}

fn write_account(output: &mut Vec<u8>, account: &AccountInfo) -> Result<(), Error> {
    write_user_value(output, &account.user);
    write_string(output, &account.home, MAX_HOME_PATH, "account home")?;
    if account.roles.len() > MAX_ROLES {
        return Err(Error::invalid_argument("account has too many roles".into()));
    }
    write_u32(output, account.roles.len() as u32);
    for role in &account.roles {
        write_string(output, role, MAX_ROLE, "account role")?;
    }

    Ok(())
}

fn read_account(reader: &mut PayloadReader<'_>) -> Result<AccountInfo, Error> {
    let user = read_user_value(reader)?;
    let home = reader.read_string(MAX_HOME_PATH, "account home")?;
    let role_count = reader.read_u32()? as usize;

    if role_count > MAX_ROLES {
        return Err(Error::invalid_argument("account has too many roles".into()));
    }

    let mut roles = Vec::new();

    for _ in 0..role_count {
        roles.push(reader.read_string(MAX_ROLE, "account role")?);
    }

    Ok(AccountInfo { user, home, roles })
}

pub fn send_default_user_request(socket: &Socket, request_id: u32) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_auth_header(&mut payload, request_id);
    socket.send_frame(AUTH_DEFAULT_USER_REQUEST, &payload)
}

pub fn send_lookup_id_request(socket: &Socket, request_id: u32, user: UserID) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_auth_header(&mut payload, request_id);
    write_u32(&mut payload, user.0);
    socket.send_frame(AUTH_LOOKUP_ID_REQUEST, &payload)
}

pub fn send_lookup_name_request(socket: &Socket, request_id: u32, name: &str) -> Result<(), Error> {
    let mut payload = Vec::new();
    write_auth_header(&mut payload, request_id);
    write_string(&mut payload, name, MAX_USER_NAME, "account name")?;
    socket.send_frame(AUTH_LOOKUP_NAME_REQUEST, &payload)
}

pub fn send_account_response(socket: &Socket, request_id: u32, status: u32, account: Option<&AccountInfo>) -> Result<(), Error> {
    let mut payload = Vec::new();

    write_auth_header(&mut payload, request_id);
    write_u32(&mut payload, status);

    if status == AUTH_STATUS_OK {
        let account = account.ok_or_else(|| Error::invalid_argument("successful auth response omitted account".into()))?;
        write_account(&mut payload, account)?;
    }

    socket.send_frame(AUTH_LOOKUP_RESPONSE, &payload)
}

pub fn recv_auth_request(socket: &Socket) -> Result<AuthRequest, Error> {
    let frame = socket.recv_frame()?;
    let mut reader = PayloadReader::new(&frame.payload);
    let request_id = read_auth_header(&mut reader)?;

    match frame.header.packet_type {
        AUTH_DEFAULT_USER_REQUEST => {
            reader.finish()?;
            Ok(AuthRequest::DefaultUser { request_id })
        }
        AUTH_LOOKUP_ID_REQUEST => {
            let user = UserID(reader.read_u32()?);
            reader.finish()?;
            Ok(AuthRequest::LookupId { request_id, user })
        }
        AUTH_LOOKUP_NAME_REQUEST => {
            let name = reader.read_string(MAX_USER_NAME, "account name")?;
            reader.finish()?;
            Ok(AuthRequest::LookupName { request_id, name })
        }
        _ => Err(Error::invalid_argument("unknown auth request type".into())),
    }
}

pub fn recv_auth_response(socket: &Socket) -> Result<AuthResponse, Error> {
    let frame = socket.recv_frame()?;
    let mut reader = PayloadReader::new(&frame.payload);
    let request_id = read_auth_header(&mut reader)?;

    match frame.header.packet_type {
        AUTH_LOOKUP_RESPONSE | AUTH_DEFAULT_USER_RESPONSE => {
            let status = reader.read_u32()?;
            let account = if status == AUTH_STATUS_OK { Some(read_account(&mut reader)?) } else { None };
            reader.finish()?;
            Ok(AuthResponse::Account { request_id, status, account })
        }
        _ => Err(Error::invalid_argument("unknown auth response type".into())),
    }
}

pub struct AuthClient {
    socket: Socket,
    next_id: u32,
}

impl AuthClient {
    pub fn connect() -> Result<Self, Error> {
        let portal_handle = resolve(&Path::new("/System/Services/Auth"), AccessRights::READ)?;
        let portal = Broker::from_handle(portal_handle);
        let socket_handle = portal.request(CAP_AUTH_CONNECT, AccessRights::READ | AccessRights::WRITE)?;

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

    pub fn default_user(&mut self) -> Result<AccountInfo, Error> {
        let request_id = self.next_id();
        send_default_user_request(&self.socket, request_id)?;
        self.recv_account(request_id)
    }

    pub fn lookup_id(&mut self, user: UserID) -> Result<AccountInfo, Error> {
        let request_id = self.next_id();
        send_lookup_id_request(&self.socket, request_id, user)?;
        self.recv_account(request_id)
    }

    pub fn lookup_name(&mut self, name: &str) -> Result<AccountInfo, Error> {
        let request_id = self.next_id();
        send_lookup_name_request(&self.socket, request_id, name)?;
        self.recv_account(request_id)
    }

    fn recv_account(&self, request_id: u32) -> Result<AccountInfo, Error> {
        match recv_auth_response(&self.socket)? {
            AuthResponse::Account { request_id: response_id, status, account } => {
                if response_id != request_id {
                    return Err(Error::invalid_argument("auth response ID mismatch".into()));
                }

                if status != AUTH_STATUS_OK {
                    return Err(match status {
                        AUTH_STATUS_NOT_FOUND => Error::not_found("account was not found".into()),
                        AUTH_STATUS_INVALID_REQUEST => Error::invalid_argument("auth rejected request".into()),
                        _ => Error::unknown("auth returned an unknown status".into()),
                    });
                }

                account.ok_or_else(|| Error::invalid_argument("successful auth response omitted account".into()))
            }
        }
    }
}

pub fn status_from_error(error: &Error) -> u32 {
    match error.kind {
        ErrorKind::NotFound => AUTH_STATUS_NOT_FOUND,
        ErrorKind::InvalidArgument | ErrorKind::InvalidEncoding | ErrorKind::NameTooLong => AUTH_STATUS_INVALID_REQUEST,
        _ => AUTH_STATUS_INTERNAL_ERROR,
    }
}
