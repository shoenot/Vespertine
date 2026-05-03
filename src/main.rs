#![no_std]
#![no_main]
use core::panic::PanicInfo;
use core::arch::asm;

mod serial;
use serial::{
    init_serial, 
    log_to_serial,
    log_u32_to_serial,
};

mod gdt;
use gdt::init_gdt;

use limine::{
    BaseRevision,
    RequestsStartMarker,
    RequestsEndMarker,
};
use limine::request::FramebufferRequest;

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(6 as u64);

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests_start")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests_end")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

fn hcf() -> ! {
    loop {
        unsafe {
            #[cfg(target_arch = "x86_64")]
            asm!("hlt");
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    if !BASE_REVISION.is_supported() {
        hcf();
    }

    unsafe {
        init_serial();
        log_to_serial("hello, world!");
        log_u32_to_serial(232839);
        init_gdt();
    }

    if let Some(fb_response) = FRAMEBUFFER_REQUEST.response() {
        if let Some(fb) = fb_response.framebuffers().first() {
            let pixels_per_row = fb.pitch / 4;
            let total_pixels = pixels_per_row * fb.height;

            let ptr = fb.address().cast::<u32>();

            for i in 0..total_pixels {
                unsafe {
                    ptr.add(i as usize).write_volatile(0x00FFFF);
                }
            }
        }
    }

    hcf();
}
