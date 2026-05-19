#![allow(dead_code)]

use core::{cmp, ptr::{
    copy,
    write_bytes,
}};

use alloc::vec::Vec;
use limine::framebuffer::Framebuffer;
use simple_psf::Psf;

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

pub fn erasechar(x: u32, y: u32, fb: &Framebuffer) {
    let x = x * 8;
    let y = y * 16;
    for xpix in x..(x+8) {
        for ypix in y..(y+16) {
            putpixel(xpix, ypix, 0x000000, &fb);
        }
    }
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

enum RenderLine {
    Append(char),
    Insert{start: usize}, // start idx
    BspcMid{start: usize, erase: usize},
    BspcEnd,
}

const MAX_COLS: usize = 256;

pub struct WriterLine {
    pub buffer: [char; MAX_COLS],
    pub len: usize,
    pub cursor: usize,
}

impl WriterLine {
    const fn new() -> Self {
        Self { buffer: ['\0'; MAX_COLS], len: 0, cursor: 0 }
    }

    fn write_char(&mut self, c: char) -> Option<RenderLine> {
        if self.len >= MAX_COLS {
            return None;
        }

        if self.cursor == self.len {
            self.buffer[self.cursor] = c;
            self.cursor += 1;
            self.len += 1;
            Some(RenderLine::Append(c))
        } else {
            for i in (self.cursor..self.len).rev() {
                self.buffer[i+1] = self.buffer[i];
            }

            self.buffer[self.cursor] = c;
            let start = self.cursor;
            self.cursor += 1;
            self.len += 1;

            Some(RenderLine::Insert { start })
        }
    }

    fn backspace(&mut self) -> Option<RenderLine> {
        if self.cursor == 0 {
            return None;
        }

        if self.cursor == self.len {
            self.cursor -= 1;
            self.len -= 1;
            Some(RenderLine::BspcEnd)
        } else {
            let erase = self.len - 1;
            for i in self.cursor..self.len {
                self.buffer[i-1] = self.buffer[i];
            }

            self.cursor -= 1;
            self.len -= 1;
            let start = self.cursor;

            Some(RenderLine::BspcMid { start, erase })
        }
    }

    pub fn cursor_back(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn cursor_fwd(&mut self) {
        if self.cursor < self.len {
            self.cursor += 1;
        }
    }
    
    pub fn clear(&mut self) {
        self.len = 0;
        self.cursor = 0;
    }
}

pub struct GraphicsWriter {
    pub current_line: u32,
    pub lim_lines: u32,
    pub current_offset: u32,
    pub max_offset: u32,
    pub font: &'static Psf<'static>,
    pub fb: SyncFramebuffer,
}

impl core::fmt::Write for GraphicsWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            match c {
                '\n' => {
                }, 
                '\x08' => { // backspace
                },
                '\x7F' => { // delete
                },
                '\x13' => { // left arrow 
                },
                '\x14' => { // right arrow 
                    self.
                },
                _ => {
                }
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

    fn inc_offset(&mut self) {
        self.erase_cursor(self.current_offset); 
        self.current_offset += 1;
        self.max_offset = cmp::max(self.max_offset, self.current_offset);
        self.draw_cursor(self.current_offset);
    }

    fn inc_till_max_offset(&mut self) {
        if self.current_offset < self.max_offset {
            self.erase_cursor(self.current_offset); 
            self.current_offset += 1;
            self.max_offset = cmp::max(self.max_offset, self.current_offset);
            self.draw_cursor(self.current_offset);
        }
    }

    fn dec_offset(&mut self) {
        self.erase_cursor(self.current_offset); 
        self.current_offset = self.current_offset.saturating_sub(1);
        self.draw_cursor(self.current_offset);
    }

    fn inc_line(&mut self) {
        self.erase_cursor(self.current_offset);
        if self.current_line < self.lim_lines {
            self.current_line += 1;
        } else {
            self.scroll();
        }
        self.current_offset = 0;
        self.max_offset = 0;
        self.draw_cursor(self.current_offset);
    }

    fn draw_cursor(&mut self, offset: u32) {
        let y = self.current_line * 16;
        let x = offset * 8;
        for ypix in y..(y+16) {
            putpixel(x, ypix, 0xFFFFFF, self.fb.0);
        }
    }

    fn erase_cursor(&mut self, offset: u32) {
        let y = self.current_line * 16;
        let x = offset * 8;
        for ypix in y..(y+16) {
            putpixel(x, ypix, 0x000000, self.fb.0);
        }
    }
}
