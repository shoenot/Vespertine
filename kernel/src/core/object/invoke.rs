use core::fmt;
use core::str::Utf8Error;

#[derive(Debug, PartialEq, Eq)]
pub enum InvocationError {
    AccessDenied,
    InvalidHandle,
    InvalidArgument,
    InvalidPointer,
    UnsupportedOperation,
    PathNotFound,
    BufferFull,
    OutOfMemory,
    WouldBlock,
    PoolExhausted,
    NameTooLong,
    InvalidEncoding,
    NotMapped,
}
impl fmt::Display for InvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessDenied => write!(f, "INVOCATION ERROR: Access denied."),
            Self::InvalidHandle => write!(f, "INVOCATION ERROR: Invalid handle."),
            Self::InvalidArgument => write!(f, "INVOCATION ERROR: Invalid argument"),
            Self::InvalidPointer => write!(f, "INVOCATION ERROR: Invalid pointer"),
            Self::UnsupportedOperation => write!(f, "INVOCATION ERROR: Unsupported operation."),
            Self::BufferFull => write!(f, "INVOCATION ERROR: Buffer full."),
            Self::PathNotFound => write!(f, "INVOCATION ERROR: Path not found."),
            Self::OutOfMemory => write!(f, "INVOCATION ERROR: Out of memory."),
            Self::WouldBlock => write!(f, "INVOCATION ERROR: Would block."),
            Self::PoolExhausted => write!(f, "INVOCATION ERROR: Pool exhausted."),
            Self::NameTooLong => write!(f, "INVOCATION ERROR: Name too long."),
            Self::InvalidEncoding => write!(f, "INVOCATION ERROR: Invalid encoding."),
            Self::NotMapped => write!(f, "INVOCATION ERROR: Not mapped."),
        }
    }
}

impl From<Utf8Error> for InvocationError {
    fn from(_: Utf8Error) -> Self { InvocationError::InvalidArgument }
}
