use alloc::format;
use alloc::string::String;
use core::fmt;
use core::str::Utf8Error;

use crate::kernel::object::handle::AccessRights;
use crate::kernel::object::op::{
    ChannelOp,
    DirectoryOp,
    FileOp
};

#[derive(Debug)]
pub enum InvocationError {
    AccessDenied,
    InvalidHandle,
    InvalidArgument(String),
    UnsupportedOperation,
}

impl fmt::Display for InvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessDenied => write!(f, "INVOCATION ERROR: Access denied."),
            Self::InvalidHandle => write!(f, "INVOCATION ERROR: Invalid handle."),
            Self::InvalidArgument(s) => write!(f, "INVOCATION ERROR: Invalid argument: {}", s),
            Self::UnsupportedOperation => write!(f, "INVOCATION ERROR: Unsupported operation."),
        }
    }
}

impl From<Utf8Error> for InvocationError {
    fn from(err: Utf8Error) -> Self { InvocationError::InvalidArgument(format!("Invalid UTF-8 bytes passed ({})", err)) }
}

#[repr(C)]
#[derive(Debug)]
pub enum Invocation {
    Ping,
    GetInfo,
    Channel(ChannelOp),
    Directory(DirectoryOp),
    File(FileOp),
}

impl Invocation {
    pub fn required_rights(&self) -> AccessRights {
        match self {
            Invocation::Ping => AccessRights::READ,
            Invocation::GetInfo => AccessRights::READ,
            Invocation::Channel(ChannelOp::PushSmall { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelOp::PushLarge { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelOp::Pull) => AccessRights::READ,
            Invocation::Directory(DirectoryOp::Link { .. }) => AccessRights::WRITE,
            Invocation::Directory(DirectoryOp::Unlink { .. }) => AccessRights::WRITE,
            Invocation::Directory(DirectoryOp::Lookup { .. }) => AccessRights::READ,
            Invocation::File(FileOp::Read { .. }) => AccessRights::READ,
        }
    }
}
