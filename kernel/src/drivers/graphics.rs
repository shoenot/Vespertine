#![allow(dead_code)]

use limine::framebuffer::Framebuffer;
use simple_psf::Psf;

// Premium Charcoal & Carbon Colors
pub const COLOR_BG: u32 = 0x11080d;       
pub const COLOR_FG: u32 = 0xe0ddd8;       
pub const COLOR_FANCY_FG: u32 = 0xad687d;       
pub const COLOR_TEAL: u32 = 0x5276b5;     // [INFO]
pub const COLOR_AMBER: u32 = 0xd9a95f;    // [WARNING]
pub const COLOR_MAGENTA: u32 = 0xc85d5d;  // [PANIC] / [FATAL]
pub const COLOR_EMERALD: u32 = 0x8eb574;  // [SUCCESS] / [OK]

fn tag_color(tag: &str) -> Option<u32> {
    match tag {
        "INFO"              => Some(COLOR_TEAL),
        "WARNING" | "WARN"  => Some(COLOR_AMBER),
        "PANIC"   | "FATAL" => Some(COLOR_MAGENTA),
        "SUCCESS" | "OK"    => Some(COLOR_EMERALD),
        "FANCY"             => Some(COLOR_FANCY_FG),
        _                   => None,
    }
}

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

pub fn putchar(c: char, x: u32, y: u32, font: &Psf, fb: &Framebuffer, fg: u32, bg: u32) {
    let x_base = (x * 8) + 16;  // 16px left margin
    let y_base = (y * 16) + 16; // 16px top margin
    let Some(pixels) = font.get_glyph_pixels(c as usize) else { return };
    pixels.enumerate().for_each(|(i, p)| {
        let px = x_base + (i as u32 % 8);
        let py = y_base + (i as u32 / 8);
        let color = if p { fg } else { bg };
        putpixel(px, py, color, fb);
    });
}

pub fn erasechar(x: u32, y: u32, fb: &Framebuffer, bg: u32) {
    let x_base = (x * 8) + 16;
    let y_base = (y * 16) + 16;
    for ypix in y_base..(y_base + 16) {
        for xpix in x_base..(x_base + 8) {
            putpixel(xpix, ypix, bg, fb);
        }
    }
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

// main dumb text processor
pub struct TextProcessor {
    pub current_row: u32,
    pub current_col: u32,
    pub max_rows: u32,
    pub max_cols: u32,
    pub fg_color: u32,
    pub bg_color: u32,
    pub font: &'static Psf<'static>,
    pub fb: SyncFramebuffer,
}

impl TextProcessor {
    pub fn draw_char(&self, c: char, col: u32, row: u32, fg: u32, bg: u32) {
        putchar(c, col, row, self.font, self.fb.0, fg, bg);
    }

    pub fn clear_cell(&self, col: u32, row: u32) {
        erasechar(col, row, self.fb.0, self.bg_color);
    }

    pub fn clear_line(&self, row: u32) {
        for col in 0..self.max_cols {
            self.clear_cell(col, row);
        }
    }

    pub fn scroll(&mut self) {
        let fb = self.fb.0;
        let pitch = fb.pitch as usize;
        let address = fb.address() as *mut u8;
        let start_y = 16;
        let scroll_amount = 16 * pitch;
        let copy_size = (self.max_rows as usize - 1) * 16 * pitch;

        unsafe {
            let dest = address.add(start_y * pitch);
            let src = dest.add(scroll_amount);
            core::ptr::copy(src, dest, copy_size);
        }
        self.clear_line(self.max_rows - 1);
    }
}

// persistent state machine
pub enum ParseState {
    Normal,
    TagCollect { buf: [u8; 16], len: usize },     // buffer holds the chars after '[' and until ']' 
    InTag { color: u32, depth: usize },
    InFancy, // fancy = text surrounded by asterisks
}

// writer wrapper over text processsor
pub struct GraphicsWriter {
    pub processor: TextProcessor,
    pub line: WriterLine,
    pub parse_state: ParseState,
    pub prompt_col: u32,
}

impl core::fmt::Write for GraphicsWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            match c {
                '\n' => {
                    self.processor.current_col = 0;
                    self.prompt_col = 0;
                    self.inc_line();
                    self.line.clear();
                    continue;
                }
                '\r' => {
                    self.processor.current_col = 0;
                    continue;
                }
                _ => {}
            }

            let state = core::mem::replace(&mut self.parse_state, ParseState::Normal);
            match state {
                ParseState::Normal => {
                    if c == '[' {
                        // start collecting a potential tag name
                        self.parse_state = ParseState::TagCollect { buf: [0u8; 16], len: 0 };
                        // dont emit '[' yet, emit it only if the tag is unrecognized
                    } else if c == '*' {
                        // enter fancy span, don't print the asterisk
                        self.parse_state = ParseState::InFancy;
                    } else {
                        self.parse_state = ParseState::Normal;
                        self.emit(c, self.processor.fg_color);
                    }
                }

                ParseState::TagCollect { mut buf, mut len } => {
                    if c == ']' {
                        let tag = core::str::from_utf8(&buf[..len]).unwrap_or("");
                        let color = tag_color(tag);
                        if let Some(color) = color {
                            self.emit('[', color);
                            for &b in &buf[..len] {
                                self.emit(b as char, color);
                            }
                            self.emit(']', color);
                            self.parse_state = ParseState::Normal;
                        } else {
                            self.emit('[', self.processor.fg_color);
                            for &b in &buf[..len] {
                                self.emit(b as char, self.processor.fg_color);
                            }
                            self.emit(']', self.processor.fg_color);
                            self.parse_state = ParseState::Normal;
                        }
                    } else if c.is_ascii_alphabetic() || c == '_' {
                        if len < buf.len() {
                            buf[len] = c as u8;
                            len += 1;
                        }
                        self.parse_state = ParseState::TagCollect { buf, len };
                    } else {
                        self.emit('[', self.processor.fg_color);
                        for &b in &buf[..len] {
                            self.emit(b as char, self.processor.fg_color);
                        }
                        self.parse_state = ParseState::Normal;
                        if c == '*' {
                            self.parse_state = ParseState::InFancy;
                        } else {
                            self.emit(c, self.processor.fg_color);
                        }
                    }
                }

                ParseState::InTag { color, depth } => {
                    self.parse_state = ParseState::InTag { color, depth };
                    self.emit(c, color);
                }

                ParseState::InFancy => {
                    if c == '*' {
                        self.parse_state = ParseState::Normal;
                    } else {
                        self.parse_state = ParseState::InFancy;
                        self.emit(c, COLOR_FANCY_FG);
                    }
                }
            }
        }
        Ok(())
    }
}

impl GraphicsWriter {
    fn emit(&mut self, c: char, fg: u32) {
        if self.processor.current_col >= self.processor.max_cols {
            self.processor.current_col = 0;
            self.inc_line();
        }
        self.processor.draw_char(c, self.processor.current_col, self.processor.current_row, fg, self.processor.bg_color);
        self.processor.current_col += 1;
    }

    pub fn inc_line(&mut self) {
        if self.processor.current_row < self.processor.max_rows - 1 {
            self.processor.current_row += 1;
        } else {
            self.processor.scroll();
        }
    }

    pub fn write_input_char(&mut self, c: char) {
        self.erase_cursor(self.prompt_col + self.line.cursor as u32);
        if let Some(action) = self.line.write_char(c) {
            self.apply_render(action);
        }
        self.draw_cursor(self.prompt_col + self.line.cursor as u32);
    }

    pub fn backspace(&mut self) {
        self.erase_cursor(self.prompt_col + self.line.cursor as u32);
        if let Some(action) = self.line.backspace() {
            self.apply_render(action);
        }
        self.draw_cursor(self.prompt_col + self.line.cursor as u32);
    }

    fn apply_render(&mut self, action: RenderLine) {
        match action {
            RenderLine::Append(c) => {
                let x = self.prompt_col + (self.line.cursor - 1) as u32;
                self.processor.draw_char(c, x, self.processor.current_row, 
                    self.processor.fg_color, self.processor.bg_color);
            }
            RenderLine::Insert { start } => {
                for i in start..self.line.len {
                    let x = self.prompt_col + i as u32;
                    self.processor.clear_cell(x, self.processor.current_row);
                    self.processor.draw_char(self.line.buffer[i], x, 
                        self.processor.current_row, 
                        self.processor.fg_color, self.processor.bg_color);
                }
            }
            RenderLine::BspcEnd => {
                let x = self.prompt_col + self.line.cursor as u32;
                self.processor.clear_cell(x, self.processor.current_row);
            }
            RenderLine::BspcMid { start, erase } => {
                for i in start..self.line.len {
                    let x = self.prompt_col + i as u32;
                    self.processor.clear_cell(x, self.processor.current_row);
                    self.processor.draw_char(self.line.buffer[i], x, 
                        self.processor.current_row,
                        self.processor.fg_color, self.processor.bg_color);
                }
                self.processor.clear_cell(
                    self.prompt_col + erase as u32, 
                    self.processor.current_row
                );
            }
        }
    }

    pub fn draw_cursor(&mut self, offset: u32) {
        let x = offset * 8 + 16;
        let y_start = self.processor.current_row * 16 + 16;
        for ypix in y_start..(y_start + 16) {
            putpixel(x, ypix, 0xFFFFFF, self.processor.fb.0);
        }
    }

    pub fn erase_cursor(&mut self, offset: u32) {
        let x = offset * 8 + 16;
        let y_start = self.processor.current_row * 16 + 16;
        for ypix in y_start..(y_start + 16) {
            putpixel(x, ypix, self.processor.bg_color, self.processor.fb.0);
        }
    }

    pub fn set_prompt_end(&mut self) {
        self.prompt_col = self.processor.current_col;
    }
}
