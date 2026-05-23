use alloc::slice;

use crate::{arch::x86_64::task::syscall::safe_copy_from, kernel::object::{handle::AccessRights, invoke::{Invocation, InvocationError}, obj::KernelObject, op::FileOp}, klogln};

#[derive(Debug)]
pub struct ConsoleWriter {}

impl KernelObject for ConsoleWriter {
    fn type_name(&self) -> &'static str {
        "Console"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Write { offset, buffer_ptr, len }) => {
                if !calling_rights.contains(AccessRights::WRITE) { return Err(InvocationError::AccessDenied) };
                self.console_log(offset, buffer_ptr, len)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl ConsoleWriter {
    pub fn console_log(&self, offset: usize, buffer_ptr: *mut u8, len: usize) -> Result<usize, InvocationError> {
        if len > 255 { return Err(InvocationError::InvalidArgument) };
        let mut console_str = [0u8; 255];
        let str_ptr = console_str.as_mut_ptr() ;

        let console_str = unsafe {
            if !safe_copy_from(str_ptr, buffer_ptr, len) {
                return Err(InvocationError::InvalidArgument);
            }
            let str_bytes = slice::from_raw_parts(str_ptr, len);
            str::from_utf8(str_bytes)?
        };

        klogln!("{}", console_str);
        Ok(0)
    }
}
