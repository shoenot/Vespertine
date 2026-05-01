#![no_std]
#![no_main]
use core::panic::PanicInfo;
use core::arch::asm;

use limine::{
    BaseRevision,
    RequestsStartMarker,
    RequestsEndMarker,
};
use limine::request::FramebufferRequest;

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".kernel_reqs")]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[repr(C)]
struct KernelRequests {
    start: RequestsStartMarker,
    fb: FramebufferRequest,
    end: RequestsEndMarker,
}

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".kernel_reqs")]
static REQUESTS: KernelRequests = KernelRequests {
    start: RequestsStartMarker::new(),
    fb: FramebufferRequest::new(),
    end: RequestsEndMarker::new(),
};

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

    if let Some(fb_response) = REQUESTS.fb.response() {
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
