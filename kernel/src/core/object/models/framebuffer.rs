use alloc::boxed::Box;
use alloc::slice;
use alloc::sync::Arc;
use core::cmp;

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    FileOp,
    Invocation,
};

use crate::arch::x86_64::task::syscall::safe_copy_to;
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::vmo::VmoObject;
use crate::core::object::obj::KernelObject;
use crate::core::sync::TicketLock;
use crate::drivers::video::FramebufferInfo;

#[derive(Debug)]
pub struct FramebufferDevice {
    pub vmo: Arc<VmoObject>,
    pub info: FramebufferInfo,
    pub offset: TicketLock<usize>,
}

#[async_trait]
impl KernelObject for FramebufferDevice {
    fn type_name(&self) -> &'static str { "Framebuffer Device" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset, buffer_ptr, len }) => {
                calling_rights.err_if_no(AccessRights::READ)?;
                let mut current_offset = offset;
                let use_cursor = offset == usize::MAX;
                if use_cursor {
                    current_offset = *self.offset.lock();
                }

                let info_bytes =
                    unsafe { slice::from_raw_parts(&self.info as *const FramebufferInfo as *const u8, size_of::<FramebufferInfo>()) };

                if current_offset >= info_bytes.len() {
                    return Ok(0);
                }

                let bytes_available = info_bytes.len() - current_offset;
                let read_len = cmp::min(bytes_available, len);
                unsafe {
                    if !safe_copy_to(buffer_ptr as *mut u8, info_bytes.as_ptr().add(current_offset), read_len) {
                        return Err(InvocationError::InvalidPointer);
                    }
                }

                if use_cursor {
                    *self.offset.lock() += read_len;
                }

                Ok(read_len)
            }
            Invocation::File(FileOp::Seek { offset, whence }) => {
                let info_size = size_of::<FramebufferInfo>() as i64;
                let mut cursor = self.offset.lock();
                let new_pos = match whence {
                    0 => offset,
                    1 => (*cursor as i64) + offset,
                    2 => info_size + offset,
                    _ => return Err(InvocationError::UnsupportedOperation),
                };

                if new_pos < 0 {
                    return Err(InvocationError::UnsupportedOperation);
                }

                *cursor = new_pos as usize;
                Ok(*cursor)
            }
            // forward vmo ops straight to the framebuffer vmo
            Invocation::Vmo(op) => self.vmo.invoke(Invocation::Vmo(op), calling_rights).await,
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
