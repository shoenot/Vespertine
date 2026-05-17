use crate::kernel::object::handle::{AccessRights, HandleID};
use core::fmt;

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
}

#[repr(C)]
#[derive(Debug)]
pub enum ChannelMessage {
    PushSmall { data: [u8; 32], len: u8 },
    PushLarge { vmo_handle: HandleID, offset: usize, len: usize },
    Pull,
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

