use core::cmp::min;

use crate::arch::x86_64::task::syscall::safe_copy_to;
use crate::core::object::invoke::InvocationError;
use vespertine_abi::Invocation;
use crate::core::object::obj::KernelObject;
use vespertine_abi::op::FileOp;
use vespertine_abi::AccessRights;

#[repr(C)]
#[derive(Debug)] 
pub struct FileObj {
    addr: *const u8,
    size: usize,
}

unsafe impl Send for FileObj {}
unsafe impl Sync for FileObj {}

impl KernelObject for FileObj {
    fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset, buffer_ptr, len }) => { self.read_file(offset, buffer_ptr, len) },
            Invocation::File(FileOp::Stat) => self.stat(),
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str {
        "File"
    }
}

impl FileObj {
    pub const fn new(addr: *const u8, size: usize) -> Self {
        Self { addr, size }
    }

    // unix behavior: returns 0 if there's nothing to read 
    fn read_file(&self, offset: usize, buffer_ptr: *mut u8, req_len: usize) -> Result<usize, InvocationError> {
        if offset >= self.size {
            return Ok(0);
        }
        let bytes_available = self.size - offset;
        let read_len = min(bytes_available, req_len);
        if read_len == 0 {
            return Ok(0);
        }

        unsafe { 
            let ptr = self.addr.add(offset);
            if !safe_copy_to(buffer_ptr, ptr, read_len) { return Err(InvocationError::InvalidArgument) };
        }
        Ok(read_len)
    }

    fn stat(&self) -> Result<usize, InvocationError> {
        Ok(self.size)
    }
}
