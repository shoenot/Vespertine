use core::fmt;

use alloc::string::String;

use crate::kernel::object::handle::{
    AccessRights,
    HandleID,
};

#[derive(Debug)]
pub enum InvocationError {
    AccessDenied,
    InvalidHandle,
    UnsupportedOperation,
}

impl fmt::Display for InvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessDenied => write!(f, "INVOCATION ERROR: Access denied."),
            Self::InvalidHandle => write!(f, "INVOCATION ERROR: Invalid handle."),
            Self::UnsupportedOperation => write!(f, "INVOCATION ERROR: Unsupported operation."),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub enum Invocation {
    Ping,
    GetInfo,
    Channel(ChannelMessage),
    Directory(DirectoryMessage),
}

#[repr(C)]
#[derive(Debug)]
pub enum ChannelMessage {
    PushSmall { data: [u8; 32], len: u8 },
    PushLarge { vmo_handle: HandleID, offset: usize, len: usize },
    Pull,
}

#[repr(C)]
#[derive(Debug)]
pub enum DirectoryMessage {
    Link { name: String, handle_id: HandleID },
    Unlink { name: String },
    Lookup { name: String },
}

impl Invocation {
    pub fn required_rights(&self) -> AccessRights {
        match self {
            Invocation::Ping => AccessRights::READ,
            Invocation::GetInfo => AccessRights::READ,
            Invocation::Channel(ChannelMessage::PushSmall { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelMessage::PushLarge { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelMessage::Pull) => AccessRights::READ,
        }
    }
}
