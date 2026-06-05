use alloc::sync::Arc;

use limine::framebuffer::Framebuffer;

use crate::boot::FRAMEBUFFER_REQUEST;
use crate::core::object::models::framebuffer::FramebufferDevice;
use crate::core::object::models::vmo::VmoObject;
use crate::core::sync::TicketLock;
use crate::drivers::logger;
use crate::memory::HHDMOFFSET;
use crate::memory::vmo::Vmo;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    width: usize,
    height: usize,
    pitch: usize,
    bpp: usize,
}

fn get_framebuffer() -> &'static Framebuffer {
    if let Some(fb_response) = FRAMEBUFFER_REQUEST.response() {
        if let Some(fb) = fb_response.framebuffers().first() {
            return *fb;
        }
    };
    panic!("CANNOT GET FRAMEBUFFER");
}

pub fn init_framebuffer() -> FramebufferDevice {
    logger::disable_screen_logging();
    let fb = get_framebuffer();
    let height = fb.height as usize;
    let width = fb.width as usize;
    let pitch = fb.pitch as usize;
    let bpp = fb.bpp as usize;
    let fb_phys_addr = (fb.address() as usize) - *HHDMOFFSET;
    let fb_size = (pitch as usize) * (height as usize);

    let fb_vmo = Vmo::new_phys(fb_phys_addr, fb_size);

    let fb_vmo_obj = Arc::new(VmoObject::new(fb_vmo));
    let info = FramebufferInfo { width, height, pitch, bpp };
    FramebufferDevice { vmo: fb_vmo_obj, info, offset: TicketLock::new(0) }
}
