#![no_std]
#![no_main]
use core::panic::PanicInfo; 
use core::arch::asm;
use simple_psf::Psf;
use simple_psf::ParseError;

mod arch;
mod drivers;
mod kernel;

use drivers::serial::{
    init_serial, 
    log_to_serial,
    log_u32_to_serial,
};

use arch::x86_64::interrupts::gdt::init_gdt;
use arch::x86_64::interrupts::idt::init_idt;

use drivers::graphics::*;

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

const FONT_DATA: &[u8] = include_bytes!("../zap-ext-light16.psf");

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    if !BASE_REVISION.is_supported() {
        hcf();
    }

    unsafe {
        init_serial();
        log_to_serial("\x1B[2J\x1B[H");
        log_to_serial("INITIATING GDT... ");
        init_gdt();
        log_to_serial("INITIATING IDT... ");
        init_idt();
        log_to_serial("hello, world!\n");
    }

    let font = match Psf::parse(FONT_DATA) {
        Ok(f) => f,
        Err(ParseError::HeaderMissing) => { log_to_serial("FONT LOAD FAILED: HEADER MISSING"); hcf() },
        Err(ParseError::InvalidMagicBytes) => { log_to_serial("FONT LOAD FAILED: INVALID MAGIC BYTES"); hcf() },
        Err(ParseError::UnknownVersion(_)) => { log_to_serial("FONT LOAD FAILED: UNKNOWN VERSION"); hcf() },
        Err(ParseError::GlyphTableTruncated {..}) => { log_to_serial("FONT LOAD FAILED: GLYPH TABLE TRUNCATED"); hcf() },
    };
    log_to_serial("FONT LOADED\n");

    let fb = if let Some(fb_response) = FRAMEBUFFER_REQUEST.response() {
        if let Some(fb) = fb_response.framebuffers().first() {
            fb
        } else { log_to_serial("Cannot get framebuffer"); hcf() }
    } else { log_to_serial("Cannot get framebuffer"); hcf() };

    writeline("Hello, world!", 0, &font, fb);

    hcf();
}
