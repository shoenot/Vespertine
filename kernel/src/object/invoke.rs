use core::fmt;
use core::str::Utf8Error;

use crate::executor::waiter::{Terminating, TerminationToken, interruptible_wait};
use crate::memory::vmm::VmError;
use crate::sched::ThreadError;

#[derive(Clone, Copy)] 
pub struct InvokeContext<'a> {
    termination: Option<&'a TerminationToken>,
    user_pointers: bool, 
}

impl<'a> InvokeContext<'a> {
    pub fn for_thread(token: &'a TerminationToken) -> Self {
        Self { termination: Some(token), user_pointers: true }
    }

    pub fn kernel() -> Self {
        Self { termination: None, user_pointers: false }
    }

    pub async fn wait<T, F>(self, future: F) 
        -> Result<T, InvocationError> where F: Future<Output = Result<T, InvocationError>> {
            match self.termination {
                Some(token) => interruptible_wait(token, future).await?,
                None => future.await,
            }
    }

    pub fn uses_user_pointers(self) -> bool {
        self.user_pointers
    }
}


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
    ThreadSpawnFail,
    NotFound,
    Io,
    NoSpace,
    Terminating,
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
            Self::ThreadSpawnFail => write!(f, "INVOCATION ERROR: Failed to spawn thread."),
            Self::NotFound => write!(f, "INVOCATION ERROR: Not found."),
            Self::Io => write!(f, "INVOCATION ERROR: I/O error."),
            Self::NoSpace => write!(f, "INVOCATION ERROR: No space."),
            Self::Terminating => write!(f, "INVOCATION ERROR: Calling thread is terminating."),
        }
    }
}

impl From<Utf8Error> for InvocationError {
    fn from(_: Utf8Error) -> Self { InvocationError::InvalidArgument }
}

impl From<VmError> for InvocationError {
    fn from(value: VmError) -> Self {
        match value {
            VmError::AccessDenied => Self::AccessDenied,
            VmError::OutOfMemory => Self::OutOfMemory,
            _ => Self::InvalidArgument,
        }
    }
}

impl From<ThreadError> for InvocationError {
    fn from(value: ThreadError) -> Self {
        match value {
            ThreadError::AllocationFailed => Self::OutOfMemory,
            ThreadError::SpawnAllocationError(_) => Self::ThreadSpawnFail,
        }
    }
}

impl From<Terminating> for InvocationError {
    fn from(_value: Terminating) -> Self {
        Self::Terminating
    }
}
