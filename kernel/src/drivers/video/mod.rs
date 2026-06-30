use alloc::sync::Arc;
use hal::boot::{self, BootFramebuffer};

use crate::core::object::framebuffer::FramebufferDevice;
use crate::core::object::vmo::VmoObject;
use crate::sync::TicketLock;
use crate::drivers::logger;
use crate::memory::vmo::Vmo;

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
    let fb_size = fb.pitch * fb.height;

    let fb_vmo = Vmo::new_phys(fb.physical_address, fb_size);
    let fb_vmo_obj = Arc::new(VmoObject::new(fb_vmo));

    let info = FramebufferInfo { width: fb.width, height: fb.height, pitch: fb.pitch, bpp: fb.bpp };
    FramebufferDevice { vmo: fb_vmo_obj, info, offset: TicketLock::new(0) }
}
