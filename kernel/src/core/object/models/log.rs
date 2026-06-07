use async_trait::async_trait;
use vespertine_abi::{AccessRights, FileOp, Invocation};

use crate::{arch::x86_64::task::syscall::safe_copy_from, core::object::{invoke::InvocationError, obj::KernelObject}, drivers::logger::LOGGER, klogln};

use alloc::boxed::Box;

#[derive(Debug)]
pub struct Log;

#[async_trait]
impl KernelObject for Log {
    fn type_name(&self) -> &'static str { "System Log" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Write { offset: _, buffer_ptr, len }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }

                if len > 1024 {
                    return Err(InvocationError::BufferFull);
                }

                let mut buf = [0u8; 1024];
                if !safe_copy_from(buf.as_mut_ptr(), buffer_ptr as *const u8, len) {
                    return Err(InvocationError::InvalidPointer);
                }
                if let Ok(s) = str::from_utf8(&buf[..len]) {
                    klogln!("{}", s);
                }
                Ok(len)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
