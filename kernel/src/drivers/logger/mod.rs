use alloc::boxed::Box;
use hal::boot::{self, BootFramebuffer};
use core::fmt::{
    self,
    Write,
};
use core::mem::MaybeUninit;
use core::{
    ptr,
    str,
};

use async_trait::async_trait;
use hal::usercopy::safe_copy_from;
use simple_psf::Psf;
use vespertine_abi::op::FileOp;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use super::serial::{
    SerialWriter,
    init_serial,
    log_to_serial,
};
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::core::sync::{
    KernelOnceCell,
    TicketLock,
};

pub const COLOR_BG: u32 = 0x11080d;
pub const COLOR_FG: u32 = 0xe0ddd8;

const FONT_DATA: &[u8] = include_bytes!("zap-ext-light16.psf");
static FONT: KernelOnceCell<Psf<'static>> = KernelOnceCell::new();

fn load_font() -> Psf<'static> {
    match Psf::parse(FONT_DATA) {
        Ok(f) => f,
        Err(_) => panic!("FONT LOAD FAILED"),
    }
}

pub static LOGGER: TicketLock<Logger> = TicketLock::new(Logger {
    serial_writer: MaybeUninit::uninit(),
    screen_enabled: true,
    current_row: 0,
    current_col: 0,
    max_rows: 0,
    max_cols: 0,
});

#[derive(Debug)]
pub struct Logger {
    pub serial_writer: MaybeUninit<SerialWriter>,
    pub screen_enabled: bool,
    pub current_row: u32,
    pub current_col: u32,
    pub max_rows: u32,
    pub max_cols: u32,
}

impl Logger {
    pub fn init(&mut self) {
        init_serial();
        log_to_serial("\x1B[2J\x1B[H");
        self.serial_writer.write(SerialWriter {});

        if let Some(fb) = hal::boot::framebuffer() {
            let total_pixels = (fb.pitch / 4) * fb.height;
            let ptr = fb.virtual_address as *mut u32;
            unsafe {
                for i in 0..total_pixels {
                    ptr.add(i).write_volatile(COLOR_BG);
                }
            }
            self.max_rows = ((fb.height - 32) / 16) as u32;
            self.max_cols = ((fb.width - 32) / 8) as u32;
            self.current_row = 0;
            self.current_col = 0;
        }
    }

    pub fn write_serial_only(&mut self, s: &str) -> fmt::Result { unsafe { self.serial_writer.assume_init_mut().write_str(s) } }

    pub fn write_screen(&mut self, s: &str) {
        let Some(fb) = boot::framebuffer() else { return };
        let font = FONT.get_or_init(|| load_font());

        for c in s.chars() {
            if c == '\n' {
                self.current_col = 0;
                self.inc_line(fb);
                continue;
            }
            if c == '\r' {
                self.current_col = 0;
                continue;
            }

            if self.current_col >= self.max_cols {
                self.current_col = 0;
                self.inc_line(fb);
            }

            putchar(c, self.current_col, self.current_row, font, fb, COLOR_FG, COLOR_BG);
            self.current_col += 1;
        }
    }

    fn inc_line(&mut self, fb: BootFramebuffer) {
        if self.current_row < self.max_rows - 1 {
            self.current_row += 1;
        } else {
            self.scroll(fb);
        }
    }

    fn scroll(&mut self, fb: BootFramebuffer) {
        let pitch = fb.pitch as usize;
        let address = fb.virtual_address as *mut u8;
        let start_y = 16;
        let scroll_amount = 16 * pitch;
        let copy_size = (self.max_rows as usize - 1) * 16 * pitch;

        unsafe {
            let dest = address.add(start_y * pitch);
            let src = dest.add(scroll_amount);
            ptr::copy(src, dest, copy_size);
        }

        // clear the last line
        let last_row_y_start = (self.max_rows - 1) * 16 + 16;
        for ypix in last_row_y_start..(last_row_y_start + 16) {
            for xpix in 16..(16 + self.max_cols * 8) {
                putpixel(xpix, ypix, COLOR_BG, fb);
            }
        }
    }
}

impl Write for Logger {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // always write to serial
        unsafe {
            self.serial_writer.assume_init_mut().write_str(s)?;
        }

        if self.screen_enabled {
            self.write_screen(s);
        }
        Ok(())
    }
}

pub fn disable_screen_logging() { LOGGER.lock().screen_enabled = false; }

fn putpixel(x: u32, y: u32, color: u32, fb: BootFramebuffer) {
    let pixels_per_row = fb.pitch / 4;
    let ptr = fb.virtual_address as *mut u32;

    if x < fb.width as u32 && y < fb.height as u32 {
        unsafe {
            ptr.add((y * pixels_per_row as u32 + x) as usize).write_volatile(color);
        }
    }
}

fn putchar(c: char, col: u32, row: u32, font: &Psf, fb: BootFramebuffer, fg: u32, bg: u32) {
    let x_base = (col * 8) + 16; // 16px left margin
    let y_base = (row * 16) + 16; // 16px top margin
    let Some(pixels) = font.get_glyph_pixels(c as usize) else { return };
    pixels.enumerate().for_each(|(i, p)| {
        let px = x_base + (i as u32 % 8);
        let py = y_base + (i as u32 / 8);
        let color = if p { fg } else { bg };
        putpixel(px, py, color, fb);
    });
}

// boot time minimal screenwriter object
#[derive(Debug)]
pub struct ScreenWriter {}

#[async_trait]
impl KernelObject for ScreenWriter {
    fn type_name(&self) -> &'static str { "ScreenWriter" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Write { offset: _, buffer_ptr, len }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;

                if len > 1024 {
                    return Err(InvocationError::BufferFull);
                }

                let mut buf = [0u8; 1024];
                if !safe_copy_from(buf.as_mut_ptr(), buffer_ptr as *const u8, len) {
                    return Err(InvocationError::InvalidPointer);
                }
                if let Ok(s) = str::from_utf8(&buf[..len]) {
                    let mut logger = LOGGER.lock();
                    logger.write_screen(s);
                }
                Ok(len)
            }
            _ => Err(InvocationError::UnsupportedOperation),
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
        fn write_str(&mut self, s: &str) -> core::fmt::Result { self.0.write_serial_only(s) }
    }

    let mut logger = LOGGER.lock();
    let mut formatter = SerialOnlyFormatter(&mut *logger);
    core::fmt::write(&mut formatter, args).unwrap();
}
