use alloc::boxed::Box;
use alloc::sync::Arc;
use core::cmp::min;

use async_trait::async_trait;
use hal::usercopy::safe_copy_to;
use vespertine_abi::op::FileOp;
use vespertine_abi::{
    AccessRights,
    FileStat,
    Invocation,
    ObjectType,
};

use crate::object::invoke::InvocationError;
use crate::object::vmo::{VmoObject, install_private_file_vmo, new_private_file_vmo};
use crate::object::obj::KernelObject;
use crate::sync::TicketLock;
use crate::process::current_process;
use crate::memory::vmo::AnonymousVmo;
use crate::memory::PAGE_SIZE_4K;

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
    fn object_type(&self) -> ObjectType { ObjectType::File }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
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
            },
            Invocation::File(FileOp::Stat { stat_ptr }) => {
                let stat = FileStat {
                    object_type: ObjectType::File as u32,
                    mode: 0x8000 | 0o444,
                    user: 0,
                    _group: 0,
                    inode: self.addr as u64,
                    device: 0,
                    size: self.size as u64,
                    block_size: PAGE_SIZE_4K as u32,
                    blocks: self.size.div_ceil(512) as u64,
                    nlink: 1,
                    atime_sec: 0,
                    atime_nsec: 0,
                    mtime_sec: 0,
                    mtime_nsec: 0,
                    ctime_sec: 0,
                    ctime_nsec: 0,
                };

                if !safe_copy_to(stat_ptr as *mut u8, &stat as *const _ as *const u8, size_of::<FileStat>()) {
                    return Err(InvocationError::InvalidPointer);
                }
                Ok(0)
            },
            Invocation::File(FileOp::GetVmo) => {
                let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
                let backing = proc.default_pool.allocate_backing(self.size)?;
                self.copy_file_into_vmo(0, self.size, &backing).await?;

                let access = AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE;
                let rights = AccessRights::MUTATE | AccessRights::CREATE | (calling_rights & access);
                let handle = proc.handles.write().insert(Arc::new(VmoObject::new(backing)), rights);
                Ok(handle.0)
            },
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
            },
            Invocation::File(FileOp::GetPrivateVmo { offset, len }) => {
                let backing = new_private_file_vmo(offset, len, self.size)?;

                for dst_offset in (0..len).step_by(PAGE_SIZE_4K) {
                    let src_offset = offset + dst_offset;
                    let bytes = (self.size - src_offset).min(PAGE_SIZE_4K);
                    let frame = backing.get_page(dst_offset)
                        .map_err(|_| InvocationError::OutOfMemory)?;

                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            self.addr.add(src_offset),
                            frame.virt() as *mut u8,
                            bytes,
                        );
                    }
                }

                install_private_file_vmo(backing, calling_rights)
            },
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

    async fn copy_file_into_vmo(&self, offset: usize, bytes: usize, backing: &AnonymousVmo) -> Result<(), InvocationError> {
        let page = PAGE_SIZE_4K;
        if offset % page != 0 || offset.checked_add(bytes).is_none_or(|end| end > self.size) {
            return Err(InvocationError::InvalidArgument);
        }

        for dst_offset in (0..bytes).step_by(page) {
            let count = (bytes - dst_offset).min(page);
            let frame = backing.get_page(dst_offset)
                .map_err(|_| InvocationError::OutOfMemory)?;

            unsafe {
                core::ptr::copy_nonoverlapping(self.addr.add(offset + dst_offset), frame.virt() as *mut u8, count);
            }
        }

        Ok(())
    }
}
