use core::cmp::min;
use core::ptr::copy_nonoverlapping;

use crate::kernel::object::handle::AccessRights;
use crate::kernel::object::invoke::{Invocation, InvocationError};
use crate::kernel::object::obj::KernelObject;
use crate::kernel::object::op::FileOp;

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
    fn read_file(&self, offset: usize, bufer_ptr: *mut u8, req_len: usize) -> Result<usize, InvocationError> {
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
            copy_nonoverlapping(ptr, bufer_ptr, read_len);
        }
        Ok(read_len)
    }
}
