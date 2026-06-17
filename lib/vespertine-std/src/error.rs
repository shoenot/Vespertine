use core::fmt::Display;

use vespertine_rt::syscall::SysError;
extern crate alloc; use alloc::string::String;

use crate::fs::PathError;

#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, PartialEq, Clone)]
pub enum ErrorKind {
    NotFound,
    AccessDenied,
    InvalidArgument,
    InvalidHandle,
    InvalidPointer,
    OutOfMemory,
    BrokenSocket,
    WouldBlock,
    BufferFull,
    PoolExhausted,
    NameTooLong,
    InvalidEncoding,
    NotMapped,
    UnsupportedOperation,
    PathEmpty,
    PathContainsNull,
    Unknown,
}

impl From<SysError> for Error {
    fn from(e: SysError) -> Self {
        let kind = match e {
            SysError::InvalidPointer => ErrorKind::InvalidPointer,
            SysError::BadAddress => ErrorKind::NotFound,
            SysError::InvalidHandle => ErrorKind::NotFound,
            SysError::AccessDenied => ErrorKind::AccessDenied,
            SysError::InvalidArgument => ErrorKind::InvalidArgument,
            SysError::OutOfMemory => ErrorKind::OutOfMemory,
            SysError::UnsupportedOperation => ErrorKind::UnsupportedOperation,
            SysError::WouldBlock => ErrorKind::WouldBlock,
            SysError::BufferFull => ErrorKind::BufferFull,
            SysError::PoolExhausted => ErrorKind::PoolExhausted,
            SysError::NameTooLong => ErrorKind::NameTooLong,
            SysError::InvalidEncoding => ErrorKind::InvalidEncoding,
            SysError::NotMapped => ErrorKind::NotMapped,
            _ => ErrorKind::Unknown,
        };
        Error {
            kind,
            message: "".into(),
        }
    }
}

impl From<PathError> for Error {
    fn from(e: PathError) -> Self {
        let kind = match e {
            PathError::Empty        => ErrorKind::PathEmpty,
            PathError::ContainsNull => ErrorKind::PathContainsNull,
            PathError::NoFileName   => ErrorKind::InvalidArgument,
            PathError::NameTooLong  => ErrorKind::NameTooLong,
        };
        Error {
            kind,
            message: "".into(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error {
    pub fn new(kind: ErrorKind, message: String) -> Self {
        Self { kind, message }
    }
}

macro_rules! error_constructors {
    (
        $(
            $fn_name:ident => $kind:ident
        ),* $(,)?
    ) => {
        
        impl Error {
            $(

                pub fn $fn_name(message: String) -> Self {
                    Self { kind: ErrorKind::$kind, message }
                }
            )*
        }
    };
}

error_constructors! {
    not_found => NotFound,
    access_denied => AccessDenied,
    invalid_argument => InvalidArgument,
    invalid_handle => InvalidHandle,
    invalid_pointer => InvalidPointer,
    out_of_memory => OutOfMemory,
    broken_socket => BrokenSocket,
    would_block => WouldBlock,
    buffer_full => BufferFull,
    pool_exhausted => PoolExhausted,
    name_too_long => NameTooLong,
    invalid_encoding => InvalidEncoding,
    not_mapped => NotMapped,
    unsupported_operation => UnsupportedOperation,
    path_empty => PathEmpty,
    path_contains_null => PathContainsNull,
    unknown => Unknown,
}


