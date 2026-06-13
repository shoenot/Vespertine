use core::slice;

use vespertine_abi::{AccessRights, HandleID, Invocation, VmoOp};
use vespertine_rt::syscall::{sys_close, sys_invoke, sys_read};

use crate::{Error, env, fs::walk_path};

#[repr(C)]
pub struct FramebufferInfo {
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: usize,
}

pub struct Framebuffer {
    pub handle: HandleID,
    pub info: FramebufferInfo,
    pub pixel_ptr: *mut u32,
    pub size_in_bytes: usize,
}

impl Framebuffer {
    pub fn open() -> Result<Self, Error> {
        let handle = walk_path("/Devices/Framebuffer", AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE )?;

        let mut info = FramebufferInfo {
            width: 0,
            height: 0,
            pitch: 0,
            bpp: 0,
        };
        let info_ptr = &mut info as *mut _ as *mut u8;
        sys_read(handle, info_ptr, size_of::<FramebufferInfo>(), 0)?;

        let size_in_bytes = info.pitch * info.height;
        let map_op = VmoOp::MapIntoProc {
            vaddr: 0,
            len: size_in_bytes,
            vm_flags: 5,
        };
        let mapped_virt = sys_invoke(handle, &Invocation::Vmo(map_op))?;

        Ok(Self {
            handle,
            info,
            pixel_ptr: mapped_virt as *mut u32,
            size_in_bytes,
        })
    }

    pub fn pixels(&self) -> &[u32] {
        unsafe { slice::from_raw_parts(self.pixel_ptr, self.size_in_bytes / 4) }
    }

    pub fn pixels_mut(&self) -> &mut [u32] {
        unsafe { slice::from_raw_parts_mut(self.pixel_ptr, self.size_in_bytes / 4) }
    }

    pub fn info(&self) -> &FramebufferInfo {
        &self.info
    }

    #[inline(always)]
    pub fn write_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.info.width && y < self.info.height {
            let offset = y * (self.info.pitch / 4) + x;
            unsafe {
                self.pixel_ptr.add(offset).write_volatile(color);
            }
        }
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}
