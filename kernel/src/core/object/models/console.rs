use alloc::slice;
use core::fmt::write;
use core::fmt::Write;

use crate::{arch::x86_64::task::syscall::safe_copy_from, core::object::{invoke::InvocationError, obj::KernelObject}, drivers::logger::LOGGER, klogln};
use vespertine_abi::Invocation;

use vespertine_abi::op::FileOp;
use vespertine_abi::AccessRights;

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
    pub fn console_log(&self, _offset: usize, buffer_ptr: *mut u8, len: usize) -> Result<usize, InvocationError> {
        if len > 1024 { return Err(InvocationError::BufferFull) };
        let mut console_str = [0u8; 1024];
        let str_ptr = console_str.as_mut_ptr() ;

        let console_str = unsafe {
            if !safe_copy_from(str_ptr, buffer_ptr, len) {
                return Err(InvocationError::InvalidPointer);
            }
            let str_bytes = slice::from_raw_parts(str_ptr, len);
            str::from_utf8(str_bytes)?
        };

        {
            let mut logger = LOGGER.lock();
            let writer = unsafe { logger.graphics_writer.assume_init_mut() };
            let _ = writer.write_str(console_str);
            writer.set_prompt_end();
        }
        Ok(0)
    }
}
