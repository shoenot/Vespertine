use core::fmt::{
    self,
    Write,
};
use core::mem::MaybeUninit;

use limine::framebuffer::Framebuffer;
use simple_psf::Psf;

use super::graphics::{
    GraphicsWriter,
    SyncFramebuffer,
};
use super::serial::{
    SerialWriter,
    init_serial,
    log_to_serial,
};
use crate::boot::FRAMEBUFFER_REQUEST;
use crate::kernel::sync::{
    KernelOnceCell,
    TicketLock,
};

const FONT_DATA: &[u8] = include_bytes!("../../build_deps/zap-ext-light16.psf");
static FONT: KernelOnceCell<Psf<'static>> = KernelOnceCell::new();
pub static LOGGER: TicketLock<Logger> =
    TicketLock::new(Logger { graphics_writer: MaybeUninit::uninit(), serial_writer: MaybeUninit::uninit() });

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

pub struct Logger {
    pub graphics_writer: MaybeUninit<GraphicsWriter>,
    pub serial_writer: MaybeUninit<SerialWriter>,
}

impl Logger {
    pub fn init(&mut self) {
        init_serial();
        log_to_serial("\x1B[2J\x1B[H");
        let fb = get_framebuffer();
        self.graphics_writer.write(GraphicsWriter {
            current_line: 0,
            lim_lines: ((fb.height / 16) - 2) as u32,
            current_offset: 0,
            max_offset: 0,
            font: FONT.get_or_init(|| load_font()),
            fb: SyncFramebuffer(fb),
        });
        self.serial_writer.write(SerialWriter {});
    }
}

impl Write for Logger {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Write to both outputs
        unsafe {
            self.graphics_writer.assume_init_mut().write_str(s)?;
            self.serial_writer.assume_init_mut().write_str(s)?;
            Ok(())
        }
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
