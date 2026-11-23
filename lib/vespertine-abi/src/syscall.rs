use core::fmt::Display;

#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysError {
    Success = 0,

    // Memory and pointer Errors
    InvalidPointer = 1,
    BadAddress = 2,
    OutOfMemory = 3,

    // Handle and Capability Errors
    InvalidHandle = 21,
    AccessDenied = 22,
    InvalidArgument = 23,
    UnsupportedOperation = 24,
    BufferFull = 25,
    WouldBlock = 26,
    PoolExhausted = 27,
    NameTooLong = 28,
    InvalidEncoding = 29,
    NotMapped = 30,
    NotFound = 31,
    Io = 32,
    NoSpace = 33,

    // System Errors
    UnknownSyscall = 41,

    ThreadSpawnFail = 50,
}

impl Display for SysError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SysError::Success => write!(f, "SYSCALL SUCCESS"),

            SysError::InvalidPointer => write!(f, "SYSCALL ERROR: Invalid pointer"),
            SysError::BadAddress => write!(f, "SYSCALL ERROR: Bad address"),
            SysError::OutOfMemory => write!(f, "SYSCALL ERROR: Out of memory"),

            SysError::InvalidHandle => write!(f, "SYSCALL ERROR: Invalid handle"),
            SysError::AccessDenied => write!(f, "SYSCALL ERROR: Access denied"),
            SysError::InvalidArgument => write!(f, "SYSCALL ERROR: Invalid argument"),
            SysError::UnsupportedOperation => write!(f, "SYSCALL ERROR: Unsupported operation"),
            SysError::BufferFull => write!(f, "SYSCALL ERROR: Buffer full"),
            SysError::WouldBlock => write!(f, "SYSCALL ERROR: IO operation would block right now"),
            SysError::PoolExhausted => write!(f, "SYSCALL ERROR: Memory pool exhausted"),
            SysError::NameTooLong => write!(f, "SYSCALL ERROR: Name too long"),
            SysError::InvalidEncoding => write!(f, "SYSCALL ERROR: Invalid encoding"),
            SysError::NotMapped => write!(f, "SYSCALL ERROR: Not mapped"),
            SysError::NotFound => write!(f, "SYSCALL ERROR: Not found"),
            SysError::Io => write!(f, "SYSCALL ERROR: I/O error"),
            SysError::NoSpace => write!(f, "SYSCALL ERROR: No space"),

            SysError::UnknownSyscall => write!(f, "SYSCALL ERROR: Unknown syscall"),

            SysError::ThreadSpawnFail => write!(f, "SYSCALL ERROR: Thread spawn failed"),
        }
    }
}

impl SysError {
    pub fn from_raw(status: usize) -> Self {
        match status {
            0 => SysError::Success,
            1 => SysError::InvalidPointer,
            2 => SysError::BadAddress,
            3 => SysError::OutOfMemory,
            21 => SysError::InvalidHandle,
            22 => SysError::AccessDenied,
            23 => SysError::InvalidArgument,
            24 => SysError::UnsupportedOperation,
            25 => SysError::BufferFull,
            26 => SysError::WouldBlock,
            27 => SysError::PoolExhausted,
            28 => SysError::NameTooLong,
            29 => SysError::InvalidEncoding,
            30 => SysError::NotMapped,
            31 => SysError::NotFound,
            32 => SysError::Io,
            33 => SysError::NoSpace,
            41 => SysError::UnknownSyscall,
            50 => SysError::ThreadSpawnFail,
            _ => SysError::UnknownSyscall,
        }
    }
}
