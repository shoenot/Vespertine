use limine::framebuffer::Framebuffer;
use simple_psf::Psf;

use crate::drivers::serial::{log_to_serial, log_u32_to_serial};

pub fn putpixel(x: u32, y: u32, color: u32, fb: &Framebuffer) -> Option<u32> {
    let pixels_per_row = fb.pitch / 4;
    let ptr = fb.address().cast::<u32>();
    
    if x >= fb.width as u32 || y >= fb.height as u32 { return None };

    unsafe {
        ptr.add((y * pixels_per_row as u32 + x) as usize).write_volatile(color);
    }
    Some(color)
}

pub fn putchar(c: char, x: u32, y: u32, font: &Psf, fb: &Framebuffer) {
    let x = x * 8;
    let y = y * 16;
    let Some(pixels) = font.get_glyph_pixels(c as usize) else { return };
    pixels.enumerate()
        .for_each(|(i, p)| {
            let x = x + (i as u32 % 8);
            let y = y + (i as u32 / 8);
            if p {
                putpixel(x, y, 0xFFFFFF, &fb);
            } else {};
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
