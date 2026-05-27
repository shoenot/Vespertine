use core::fmt;
use core::str::Utf8Error;

#[derive(Debug)]
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
        }
    }
}

impl From<Utf8Error> for InvocationError {
    fn from(_: Utf8Error) -> Self { InvocationError::InvalidArgument }
}
