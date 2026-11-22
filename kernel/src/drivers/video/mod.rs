use alloc::sync::Arc;
use hal::boot::{self, BootFramebuffer};

use crate::object::framebuffer::FramebufferDevice;
use crate::object::vmo::VmoObject;
use crate::sync::TicketLock;
use crate::drivers::logger;
use crate::memory::vmo::DeviceVmo;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    width: usize,
    height: usize,
    pitch: usize,
    bpp: usize,
}

fn get_framebuffer() -> BootFramebuffer {
    boot::framebuffer().expect("cannot get framebuffer")
}

pub fn init_framebuffer() -> FramebufferDevice {
    logger::disable_screen_logging();

    let fb = get_framebuffer();
    let fb_size = fb.pitch.checked_mul(fb.height).expect("framebuffer size overflow");
    let fb_vmo = DeviceVmo::from_kernel_mapping(fb.virtual_address, fb_size)
        .expect("invalid framebuffer kernel mapping");
    let fb_vmo_obj = Arc::new(VmoObject::new(fb_vmo));

    let info = FramebufferInfo { width: fb.width, height: fb.height, pitch: fb.pitch, bpp: fb.bpp };
    FramebufferDevice { vmo: fb_vmo_obj, info, offset: TicketLock::new(0) }
}
