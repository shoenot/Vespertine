use alloc::vec::Vec;
use vespertine_abi::{AccessRights, FileOp, Invocation};

use crate::{arch::x86_64::task::syscall::safe_copy_to, core::{object::{invoke::InvocationError, obj::KernelObject}, sync::Mutex}};


#[derive(Debug)]
pub struct LogBuffer {
    buf: Mutex<Vec<u8>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self { buf: Mutex::new(Vec::new()) }
    }

    pub fn append(&self, s: &str) {
        self.buf.lock().extend_from_slice(s.as_bytes());
    }
}

impl KernelObject for LogBuffer {
    fn type_name(&self) -> &'static str {
        "Log Buffer"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset, buffer_ptr, len }) => {
                let buf = self.buf.lock();
                if offset >= buf.len() { return Ok(0) };
                let available = &buf[offset..];
                let n = available.len().min(len);
                if !safe_copy_to(buffer_ptr, available.as_ptr(), n) {
                    return Err(InvocationError::InvalidArgument);
                }
                Ok(n)
            },
            Invocation::File(FileOp::Stat) => {
                Ok(self.buf.lock().len())
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
