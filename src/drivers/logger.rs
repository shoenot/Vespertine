use core::{
    fmt::{
        self,
        Write,
    },
};

use lazy_static::lazy_static;
use limine::framebuffer::Framebuffer;
use simple_psf::Psf;

use super::{
    graphics::{
        GraphicsWriter,
        SyncFramebuffer,
    },
    serial::{
        SerialWriter,
        init_serial,
        log_to_serial,
    },
};
use crate::{
    boot::FRAMEBUFFER_REQUEST,
    kernel::sync::TicketLock,
};

const FONT_DATA: &[u8] = include_bytes!("../../build_deps/zap-ext-light16.psf");

fn load_font() -> Psf<'static> {
    match Psf::parse(FONT_DATA) {
        Ok(f) => f,
        Err(_) => panic!("FONT LOAD FAILED"),
    }
}

fn get_framebuffer() -> &'static Framebuffer {
    if let Some(fb_response) = FRAMEBUFFER_REQUEST.response() {
        if let Some(fb) = fb_response.framebuffers().first() {
            return *fb;
        }
    };
    panic!("CANNOT GET FRAMEBUFFER");
}

lazy_static! {
    static ref FONT: Psf<'static> = load_font();
    pub static ref LOGGER: TicketLock<Logger> = TicketLock::new(Logger::new());
}

pub struct Logger {
    pub graphics_writer: GraphicsWriter,
    pub serial_writer: SerialWriter,
}

impl Logger {
    pub fn new() -> Self {
        init_serial();
        log_to_serial("\x1B[2J\x1B[H");
        Self {
            graphics_writer: GraphicsWriter { current_line: 0, current_offset: 0, font: &FONT, fb: SyncFramebuffer(get_framebuffer()) },
            serial_writer: SerialWriter {},
        }
    }
}

impl Write for Logger {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Write to both outputs
        self.graphics_writer.write_str(s)?;
        self.serial_writer.write_str(s)?;
        Ok(())
    }
}

#[macro_export]
macro_rules! klog {
    ($($arg:tt)*) => ($crate::drivers::logger::_klog(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! klogln {
    () => ($crate::klog!("\n"));
    ($($arg:tt)*) => ($crate::klog!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _klog(args: fmt::Arguments) { LOGGER.lock().write_fmt(args).unwrap(); }
