use alloc::boxed::Box;
use alloc::sync::Arc;
use core::cmp::min;

use async_trait::async_trait;
use vespertine_abi::op::FileOp;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use crate::arch::x86_64::task::syscall::safe_copy_to;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::vmo::VmoObject;
use crate::core::object::obj::KernelObject;
use crate::core::sync::TicketLock;
use crate::core::thread::get_current_process;
use crate::memory::vmo::{
    PagedBackingStore,
    Vmo,
};
use crate::memory::{
    HHDMOFFSET,
    NORMAL_PAGE_SIZE,
};

#[repr(C)]
#[derive(Debug)]
pub struct FileObj {
    addr: *const u8,
    size: usize,
    offset: TicketLock<usize>,
}

unsafe impl Send for FileObj {}
unsafe impl Sync for FileObj {}

#[async_trait]
impl KernelObject for FileObj {
    async fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset, buffer_ptr, len }) => {
                let mut current_offset = offset;
                let use_cursor = offset == usize::MAX;
                if use_cursor {
                    current_offset = *self.offset.lock();
                }

                let bytes_read = self.read_file(current_offset, buffer_ptr as *mut u8, len)?;

                if use_cursor {
                    *self.offset.lock() += bytes_read;
                }

                Ok(bytes_read)
            }
            Invocation::File(FileOp::Stat) => self.stat(),
            Invocation::File(FileOp::GetVmo) => {
                let vmo = Vmo::new(self.size);
                let num_pages = self.size.div_ceil(NORMAL_PAGE_SIZE);
                for i in 0..num_pages {
                    let page_offset = i * NORMAL_PAGE_SIZE;
                    let pfn = vmo.request_page(page_offset).map_err(|_| InvocationError::OutOfMemory)?;
                    let dest_virt = pfn + *HHDMOFFSET;
                    let chunk = min(NORMAL_PAGE_SIZE, self.size - page_offset);
                    unsafe {
                        core::ptr::copy_nonoverlapping(self.addr.add(page_offset), dest_virt as *mut u8, chunk);
                        if chunk < NORMAL_PAGE_SIZE {
                            core::ptr::write_bytes((dest_virt + chunk) as *mut u8, 0, NORMAL_PAGE_SIZE - chunk);
                        }
                    }
                }
                let vmo_obj = Arc::new(VmoObject::new(vmo));
                let current_proc = get_current_process().ok_or(InvocationError::UnsupportedOperation)?;
                let handle_id = current_proc.proc_handles.write().insert(vmo_obj, AccessRights::all());
                Ok(handle_id.0 as usize)
            }
            Invocation::File(FileOp::Seek { offset, whence }) => {
                let mut cursor = self.offset.lock();
                let new_pos = match whence {
                    0 => offset,
                    1 => (*cursor as i64) + offset,
                    2 => (self.size as i64) + offset,
                    _ => return Err(InvocationError::UnsupportedOperation),
                };

                if new_pos < 0 {
                    return Err(InvocationError::UnsupportedOperation);
                }

                *cursor = new_pos as usize;
                Ok(*cursor)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str { "File" }
}

impl FileObj {
    pub const fn new(addr: *const u8, size: usize) -> Self { Self { addr, size, offset: TicketLock::new(0) } }

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
            if !safe_copy_to(buffer_ptr, ptr, read_len) {
                return Err(InvocationError::InvalidPointer);
            };
        }
        Ok(read_len)
    }

    fn stat(&self) -> Result<usize, InvocationError> { Ok(self.size) }
}
