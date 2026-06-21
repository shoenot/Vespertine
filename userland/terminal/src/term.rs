use alloc::format;
use alloc::vec::Vec;
use core::cmp;
use core::ptr::{
    read_volatile,
    write_volatile,
};

use vespertine_abi::HandleID;
use vespertine_abi::app::termios::*;
use vespertine_rt::syscall::sys_write_bytes;
use vespertine_std::fb::Framebuffer;
use vte::Perform;

static FONT_DATA: &[u8] = include_bytes!("zap-ext-light16.psf");
pub const PADDING_X: usize = 12;
pub const PADDING_Y: usize = 12;

#[derive(Clone, Copy)]
pub struct Cell {
    pub char: char,
    pub fg: u32,
    pub bg: u32,
}

pub struct TerminalGrid {
    pub width_chars: usize,
    pub height_chars: usize,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub cursor_visible: bool,
    pub cursor_blink_on: bool,

    pub termios: Termios,
    pub can_buffer: Vec<u8>,

    pub current_fg: u32,
    pub current_bg: u32,

    pub cells: Vec<Cell>,
    pub fb: Framebuffer,
    pub app_source: HandleID,
}

impl Perform for TerminalGrid {
    // called when a char is printed
    fn print(&mut self, c: char) {
        if self.cursor_x >= self.width_chars {
            self.cursor_x = 0;
            self.newline();
        }

        // save to virtual grid
        let idx = self.cursor_y * self.width_chars + self.cursor_x;
        self.cells[idx] = Cell { char: c, fg: self.current_fg, bg: self.current_bg };

        // blit directly to fb
        self.draw_cell(self.cursor_x, self.cursor_y);
        self.cursor_x += 1;
    }

    // called when control chars are printed
    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.newline();
            }
            b'\r' => self.cursor_x = 0,
            b'\x08' => {
                // backspace
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                    self.clear_cell(self.cursor_x, self.cursor_y);
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, _intermediates: &[u8], _ignore: bool, action: char) {
        match action {
            'm' => {
                for param in params.iter() {
                    match param {
                        [0] => {
                            self.current_fg = 0xe0ddd8;
                            self.current_bg = 0x11080d;
                        }
                        [30..=37] => {
                            self.current_fg = translate_ansi_color(param[0] - 30);
                        }
                        [40..=47] => {
                            self.current_bg = translate_ansi_color(param[0] - 40);
                        }
                        _ => {}
                    }
                }
            }
            'J' => {
                self.clear_screen();
            }
            'N' => {
                if self.cursor_x > 0 {
                    self.newline();
                    self.cursor_x = 0;
                }
            }
            'n' => {
                for param in params.iter() {
                    match param {
                        [6] => {
                            let reply = format!("\x1b[{};{}R", self.cursor_y + 1, self.cursor_x + 1);
                            let _ = sys_write_bytes(self.app_source, reply.as_bytes());
                        }
                        _ => {}
                    }
                }
            }
            'H' | 'f' => {
                // set cursor position (cup)
                let mut row = 1;
                let mut col = 1;

                let mut iter = params.iter();
                if let Some(r_param) = iter.next() {
                    if r_param[0] > 0 {
                        row = r_param[0] as usize;
                    }
                }
                if let Some(c_param) = iter.next() {
                    if c_param[0] > 0 {
                        col = c_param[0] as usize;
                    }
                }

                self.cursor_y = cmp::min(row - 1, self.height_chars - 1);
                self.cursor_x = cmp::min(col - 1, self.width_chars - 1);
            }
            'A' => {
                // cursor up (cuu)
                let n = params.iter().next().map(|p| p[0] as usize).unwrap_or(1);
                self.cursor_y = self.cursor_y.saturating_sub(n);
            }
            'B' => {
                // cursor down (cud)
                let n = params.iter().next().map(|p| p[0] as usize).unwrap_or(1);
                self.cursor_y = core::cmp::min(self.cursor_y + n, self.height_chars - 1);
            }
            'C' => {
                // cursor forward/right (cuf)
                let n = params.iter().next().map(|p| p[0] as usize).unwrap_or(1);
                self.cursor_x = core::cmp::min(self.cursor_x + n, self.width_chars - 1);
            }
            'D' => {
                // cursor backward/left (cub)
                let n = params.iter().next().map(|p| p[0] as usize).unwrap_or(1);
                self.cursor_x = self.cursor_x.saturating_sub(n);
            }
            'K' => {
                // erase in line (el)
                let mode = params.iter().next().map(|p| p[0]).unwrap_or(0);
                match mode {
                    0 => {
                        // cursor to end of line
                        for col in self.cursor_x..self.width_chars {
                            self.clear_cell(col, self.cursor_y);
                        }
                    }
                    1 => {
                        // start of line to cursor
                        for col in 0..=self.cursor_x {
                            self.clear_cell(col, self.cursor_y);
                        }
                    }
                    2 => {
                        // clear entire line
                        for col in 0..self.width_chars {
                            self.clear_cell(col, self.cursor_y);
                        }
                    }
                    _ => {}
                }
            }
            'h' => {
                for param in params.iter() {
                    match param {
                        [25] => self.cursor_visible = true, // show cursor
                        _ => {}
                    }
                }
            }
            'l' => {
                for param in params.iter() {
                    match param {
                        [25] => self.cursor_visible = false, // hide cursor
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

impl TerminalGrid {
    pub fn draw_cell(&mut self, col: usize, row: usize) {
        let cell = self.cells[row * self.width_chars + col];
        let glyph_size = 16;
        // skip header with + 4
        let glyph_offset = 4 + cell.char as usize * glyph_size;

        let x_start = PADDING_X + col * 8;
        let y_start = PADDING_Y + row * 16;

        for y in 0..16 {
            let font_byte = FONT_DATA[glyph_offset + y];
            for x in 0..8 {
                let bit_is_set = (font_byte & (0x80 >> x)) != 0;
                let color = if bit_is_set { cell.fg } else { cell.bg };
                self.fb.write_pixel(x_start + x, y_start + y, color);
            }
        }
    }

    fn clear_cell(&mut self, col: usize, row: usize) {
        let idx = row * self.width_chars + col;
        self.cells[idx] = Cell { char: ' ', fg: self.current_fg, bg: self.current_bg };

        let x_start = PADDING_X + col * 8;
        let y_start = PADDING_Y + row * 16;
        for y in 0..16 {
            for x in 0..8 {
                self.fb.write_pixel(x_start + x, y_start + y, self.current_bg);
            }
        }
    }

    fn newline(&mut self) {
        if self.cursor_y < self.height_chars - 1 {
            self.cursor_y += 1;
        } else {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        let row_cells = self.width_chars;
        let cells_len = self.cells.len();
        self.cells.copy_within(row_cells..cells_len, 0);

        let last_row_start = (self.height_chars - 1) * self.width_chars;
        for i in last_row_start..self.cells.len() {
            self.cells[i] = Cell { char: ' ', fg: self.current_fg, bg: self.current_bg };
        }

        let info = self.fb.info();
        let screen_words = info.pitch / 4;

        let dst_start = PADDING_Y * screen_words;
        let src_start = (PADDING_Y + 16) * screen_words;
        let num_lines = (self.height_chars - 1) * 16;

        unsafe {
            let ptr = self.fb.pixel_ptr;
            for y in 0..num_lines {
                let dest_row = ptr.add(dst_start + y * screen_words);
                let src_row = ptr.add(src_start + y * screen_words);
                for x in 0..screen_words {
                    write_volatile(dest_row.add(x), read_volatile(src_row.add(x)));
                }
            }
        }

        // fill bottom 16 scanlines with bg color
        unsafe {
            let ptr = self.fb.pixel_ptr;
            let last_row_y_start = PADDING_Y + (self.height_chars - 1) * 16;
            let start_word = last_row_y_start * screen_words;
            let total_words = self.fb.size_in_bytes / 4;
            for i in start_word..total_words {
                write_volatile(ptr.add(i), self.current_bg);
            }
        }
    }

    pub fn clear_screen(&mut self) {
        let _w = self.width_chars;
        let _h = self.height_chars;
        for i in 0..self.cells.len() {
            self.cells[i] = Cell { char: ' ', fg: self.current_fg, bg: self.current_bg };
        }
        let pixels = self.fb.pixels_mut();
        for p in pixels.iter_mut() {
            *p = self.current_bg;
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    pub fn draw_cursor(&mut self, show: bool) {
        if !self.cursor_visible {
            return;
        }

        let col = self.cursor_x;
        let row = self.cursor_y;

        if col >= self.width_chars || row >= self.height_chars {
            return;
        }

        let cell = self.cells[row * self.width_chars + col];
        let glyph_size = 16;
        let glyph_offset = 4 + cell.char as usize * glyph_size;

        let x_start = PADDING_X + col * 8;
        let y_start = PADDING_Y + row * 16;

        for y in 0..16 {
            let font_byte = FONT_DATA[glyph_offset + y];
            for x in 0..8 {
                let bit_is_set = (font_byte & (0x80 >> x)) != 0;
                let color = if show {
                    // inverted colors (active cursor block)
                    if bit_is_set { cell.bg } else { cell.fg }
                } else {
                    // normal colors (restore text underneath)
                    if bit_is_set { cell.fg } else { cell.bg }
                };
                self.fb.write_pixel(x_start + x, y_start + y, color);
            }
        }
    }
}

fn translate_ansi_color(index: u16) -> u32 {
    match index {
        0 => 0x1a1a1a, // black
        1 => 0xc85d5d, // red
        2 => 0x5a7548, // green
        3 => 0xf0ca93, // yellow
        4 => 0x5276b5, // blue
        5 => 0x7d4c68, // magenta
        6 => 0xad687d, // cyan
        7 => 0xe0ddd8, // white
        _ => 0xe0ddd8,
    }
}

fn blend_color(fg: u32, bg: u32, alpha: u8) -> u32 {
    let a = alpha as u32;
    let inv_a = 255 - a;

    let fg_r = (fg >> 16) & 0xff;
    let fg_g = (fg >> 8) & 0xff;
    let fg_b = fg & 0xff;

    let bg_r = (bg >> 16) & 0xff;
    let bg_g = (bg >> 8) & 0xff;
    let bg_b = bg & 0xff;

    let r = (fg_r * a + bg_r * inv_a) / 255;
    let g = (fg_g * a + bg_g * inv_a) / 255;
    let b = (fg_b * a + bg_b * inv_a) / 255;

    (r << 16) | (g << 8) | b
}
