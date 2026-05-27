use vespertine_rt::syscall::SysError;

#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: &'static str,
}

#[derive(Debug, PartialEq)]
pub enum ErrorKind {
    NotFound,
    AccessDenied,
    InvalidArgument,
    InvalidHandle,
    InvalidPointer,
    OutOfMemory,
    BrokenSocket,
    WouldBlock,
    Unknown,
}

impl From<SysError> for Error {
    fn from(e: SysError) -> Self {
        let kind = match e {
            SysError::InvalidHandle => ErrorKind::NotFound,
            SysError::AccessDenied => ErrorKind::AccessDenied,
            SysError::InvalidArgument => ErrorKind::InvalidArgument,
            SysError::OutOfMemory => ErrorKind::OutOfMemory,
            SysError::WouldBlock => ErrorKind::WouldBlock,
            _ => ErrorKind::Unknown,
        };
        Error { kind, message: "" }
    }
}
