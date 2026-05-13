#![allow(dead_code)]

use limine::framebuffer::Framebuffer;
use simple_psf::Psf;
use core::ptr::copy;
use core::ptr::write_bytes;

pub fn putpixel(x: u32, y: u32, color: u32, fb: &Framebuffer) -> Option<u32> {
    let pixels_per_row = fb.pitch / 4;
    let ptr = fb.address().cast::<u32>();

    if x >= fb.width as u32 || y >= fb.height as u32 {
        return None;
    };

    unsafe {
        ptr.add((y * pixels_per_row as u32 + x) as usize).write_volatile(color);
    }
    Some(color)
}

pub fn putchar(c: char, x: u32, y: u32, font: &Psf, fb: &Framebuffer) {
    let x = x * 8;
    let y = y * 16;
    let Some(pixels) = font.get_glyph_pixels(c as usize) else { return };
    pixels.enumerate().for_each(|(i, p)| {
        let x = x + (i as u32 % 8);
        let y = y + (i as u32 / 8);
        if p {
            putpixel(x, y, 0xFFFFFF, &fb);
        } else {
        };
    });
}

pub fn writeline(s: &str, y: u32, offset: u32, font: &Psf, fb: &Framebuffer) {
    let mut i = offset;
    for c in s.chars() {
        putchar(c, i, y, font, fb);
        i += 1;
    }
}

pub fn writenumber(mut n: u64, y: u32, offset: u32, font: &Psf, fb: &Framebuffer) {
    let mut buffer = [0u8; 24];

    let mut i = buffer.len();
    while n > 0 && i > 0 {
        i -= 1;
        buffer[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }

    let numstr = core::str::from_utf8(&buffer[i..]).unwrap();
    writeline(numstr, y, offset, font, fb);
}

pub struct SyncFramebuffer(pub &'static Framebuffer);
unsafe impl Send for SyncFramebuffer {}
unsafe impl Sync for SyncFramebuffer {}

pub struct GraphicsWriter {
    pub current_line: u32,
    pub lim_lines: u32,
    pub current_offset: u32,
    pub font: &'static Psf<'static>,
    pub fb: SyncFramebuffer,
}

impl core::fmt::Write for GraphicsWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            if c == '\n' {
                if self.current_line < self.lim_lines {
                    self.current_line += 1;
                } else {
                    self.scroll();
                }
                self.current_offset = 0;
            } else {
                putchar(c, self.current_offset, self.current_line, self.font, self.fb.0);
                self.current_offset += 1;
            }
        }
        Ok(())
    }
}

impl GraphicsWriter {
    fn scroll(&mut self) {
        let fb_base_ptr = self.fb.0.address() as *mut u8;
        let pitch = self.fb.0.pitch;

        let active_height = (self.fb.0.height / 16) * 16;

        let pixel_lines = active_height - 16;
        let block_size = pixel_lines as u64 * pitch as u64;
        
        let src = (fb_base_ptr as u64 + (16 * pitch as u64)) as *const u8;
        unsafe {
            copy(src, fb_base_ptr, block_size as usize);
        }

        let bottom_line = (fb_base_ptr as u64 + block_size) as *mut u8;
        unsafe {
            write_bytes(bottom_line, 0, (16 * pitch) as usize);
        }
    }
}
