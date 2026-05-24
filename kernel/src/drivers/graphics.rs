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
    let x = (x * 8) + 8;
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
    let x = (x * 8) + 8;
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

const MAX_COLS: usize = 128;

pub enum RenderLine {
    Append(char),
    Insert { start: usize },
    BspcEnd,
    BspcMid { start: usize, erase: usize },
}

pub struct WriterLine {
    pub buffer: [char; MAX_COLS],
    pub len: usize,
    pub cursor: usize,
}

impl WriterLine {
    pub const fn new() -> Self {
        Self { buffer: ['\0'; MAX_COLS], len: 0, cursor: 0 }
    }

    pub fn write_char(&mut self, c: char) -> Option<RenderLine> {
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
                self.buffer[i + 1] = self.buffer[i];
            }

            self.buffer[self.cursor] = c;
            let start = self.cursor;
            self.cursor += 1;
            self.len += 1;

            Some(RenderLine::Insert { start })
        }
    }

    pub fn backspace(&mut self) -> Option<RenderLine> {
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
                self.buffer[i - 1] = self.buffer[i];
            }

            self.cursor -= 1;
            self.len -= 1;
            let start = self.cursor;

            Some(RenderLine::BspcMid { start, erase })
        }
    }

    pub fn delete(&mut self) -> Option<RenderLine> {
        if self.cursor == self.len {
            return None;
        }

        let erase = self.len - 1;
        for i in self.cursor + 1..self.len {
            self.buffer[i - 1] = self.buffer[i];
        }

        self.len -= 1;
        let start = self.cursor;
        
        Some(RenderLine::BspcMid { start, erase })
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
    pub line: WriterLine,
    pub font: &'static Psf<'static>,
    pub fb: SyncFramebuffer,
}

impl core::fmt::Write for GraphicsWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            self.erase_cursor(self.line.cursor as u32);

            match c {
                '\n' => {
                    self.inc_line();
                    self.line.clear();
                }, 
                '\x08' => { // backspace
                    if let Some(render_action) = self.line.backspace() {
                        self.apply_render(render_action);
                    }
                },
                '\x7F' => { // delete
                    if let Some(render_action) = self.line.delete() {
                        self.apply_render(render_action);
                    }
                },
                '\x13' => { // left arrow 
                    self.line.cursor_back();
                },
                '\x14' => { // right arrow 
                    self.line.cursor_fwd();
                },
                _ => { // standard character
                    if let Some(render_action) = self.line.write_char(c) {
                        self.apply_render(render_action);
                    }
                }
            }

            self.draw_cursor(self.line.cursor as u32);
        }
        Ok(())
    }
}

impl GraphicsWriter {
    fn apply_render(&mut self, action: RenderLine) {
        match action {
            RenderLine::Append(c) => {
                let x = (self.line.cursor - 1) as u32;
                putchar(c, x, self.current_line, self.font, self.fb.0);
            }
            RenderLine::Insert { start } => {
                for i in start..self.line.len {
                    erasechar(i as u32, self.current_line, self.fb.0);
                    putchar(self.line.buffer[i], i as u32, self.current_line, self.font, self.fb.0);
                }
            }
            RenderLine::BspcEnd => {
                erasechar(self.line.cursor as u32, self.current_line, self.fb.0);
            }
            RenderLine::BspcMid { start, erase } => {
                for i in start..self.line.len {
                    erasechar(i as u32, self.current_line, self.fb.0);
                    putchar(self.line.buffer[i], i as u32, self.current_line, self.font, self.fb.0);
                }
                erasechar(erase as u32, self.current_line, self.fb.0);
            }
        }
    }

    fn inc_line(&mut self) {
        if self.current_line < self.lim_lines {
            self.current_line += 1;
        } else {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        let fb_base_ptr = self.fb.0.address() as *mut u8;
        let pitch = self.fb.0.pitch;
        let active_height = (self.fb.0.height / 16) * 16;
        let pixel_lines = active_height - 16;
        let block_size = pixel_lines as u64 * pitch as u64;

        let src = (fb_base_ptr as u64 + (16 * pitch as u64)) as *const u8;
        unsafe {
            core::ptr::copy(src, fb_base_ptr, block_size as usize);
        }

        let bottom_line = (fb_base_ptr as u64 + block_size) as *mut u8;
        unsafe {
            core::ptr::write_bytes(bottom_line, 0, (16 * pitch) as usize);
        }
    }

    fn draw_cursor(&mut self, offset: u32) {
        let y = self.current_line * 16;
        let x = (offset * 8) + 8;
        for ypix in y..(y + 16) {
            putpixel(x, ypix, 0xFFFFFF, self.fb.0);
        }
    }

    pub fn erase_cursor(&mut self, offset: u32) {
        let y = self.current_line * 16;
        let x = (offset * 8) + 8;
        for ypix in y..(y + 16) {
            putpixel(x, ypix, 0x000000, self.fb.0);
        }
    }
}
