mod logbuffer;
use core::fmt::{
    self,
    Write,
};
use core::mem::MaybeUninit;

use alloc::sync::Arc;
use limine::framebuffer::Framebuffer;
use simple_psf::Psf;

use super::graphics::{
    GraphicsWriter,
    SyncFramebuffer,
    TextProcessor,
    COLOR_BG,
    COLOR_FG,
    WriterLine,
};
use super::serial::{
    init_serial,
    log_to_serial,
    SerialWriter,
};
use crate::boot::FRAMEBUFFER_REQUEST;
use crate::core::sync::{
    KernelOnceCell,
    TicketLock,
};
use crate::drivers::graphics::ParseState;
pub use crate::drivers::logger::logbuffer::LogBuffer;

const FONT_DATA: &[u8] = include_bytes!("../../../../build_deps/zap-ext-light16.psf");
static FONT: KernelOnceCell<Psf<'static>> = KernelOnceCell::new();
pub static LOGGER: TicketLock<Logger> =
    TicketLock::new(Logger { graphics_writer: MaybeUninit::uninit(), serial_writer: MaybeUninit::uninit(), target: LogTarget::Graphics });

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

pub enum LogTarget {
    Graphics,
    Buffer(Arc<LogBuffer>),
}

pub struct Logger {
    pub graphics_writer: MaybeUninit<GraphicsWriter>,
    pub serial_writer: MaybeUninit<SerialWriter>,
    pub target: LogTarget,
}

impl Logger {
    pub fn init(&mut self) {
        init_serial();
        log_to_serial("\x1B[2J\x1B[H");
        let fb = get_framebuffer();
        
        // fill the entire fb with the bg color
        let total_pixels = (fb.pitch / 4) as usize * fb.height as usize;
        let ptr = fb.address() as *mut u32;
        unsafe {
            for i in 0..total_pixels {
                ptr.add(i).write_volatile(COLOR_BG);
            }
        }

        let font = FONT.get_or_init(|| load_font());
        let max_rows = (fb.height - 32) / 16; // leaving 16px top and bottom margin
        let max_cols = (fb.width - 32) / 8;   // same for left and right margin

        self.graphics_writer.write(GraphicsWriter {
            processor: TextProcessor {
                current_row: 0,
                current_col: 0,
                max_rows: max_rows as u32,
                max_cols: max_cols as u32,
                fg_color: COLOR_FG,
                bg_color: COLOR_BG,
                font,
                fb: SyncFramebuffer(fb),
            },
            parse_state: ParseState::Normal,
            line: WriterLine::new(),
            prompt_col: 0,
        });
        self.serial_writer.write(SerialWriter {});
    }

    pub fn write_screen(&mut self, s: &str) {
        unsafe {
            let _ = self.graphics_writer.assume_init_mut().write_str(s);
        }
    }

    pub fn write_serial_only(&mut self, s: &str) -> fmt::Result {
        unsafe { self.serial_writer.assume_init_mut().write_str(s) }
    }
}

impl Write for Logger {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // always write to serial
        unsafe { self.serial_writer.assume_init_mut().write_str(s)?; }

        match &self.target {
            LogTarget::Graphics => unsafe { self.graphics_writer.assume_init_mut().write_str(s)?; },
            LogTarget::Buffer(buf) => { buf.append(s); },
        }
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


#[macro_export]
macro_rules! klog_serial {
    ($($arg:tt)*) => ($crate::drivers::logger::_klog_serial(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! klogln_serial {
    () => ($crate::klog_serial!("\n"));
    ($($arg:tt)*) => ($crate::klog_serial!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _klog_serial(args: fmt::Arguments) {
    struct SerialOnlyFormatter<'a>(&'a mut Logger);
    impl<'a> core::fmt::Write for SerialOnlyFormatter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.0.write_serial_only(s)
        }
    }

    let mut logger = LOGGER.lock();
    let mut formatter = SerialOnlyFormatter(&mut *logger);
    core::fmt::write(&mut formatter, args).unwrap();
}
