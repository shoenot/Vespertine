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
    BufferFull,
    PoolExhausted,
    NameTooLong,
    InvalidEncoding,
    NotMapped,
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
            SysError::BufferFull => ErrorKind::BufferFull,
            SysError::PoolExhausted => ErrorKind::PoolExhausted,
            SysError::NameTooLong => ErrorKind::NameTooLong,
            SysError::InvalidEncoding => ErrorKind::InvalidEncoding,
            SysError::NotMapped => ErrorKind::NotMapped,
            _ => ErrorKind::Unknown,
        };
        Error { kind, message: "" }
    }
}
