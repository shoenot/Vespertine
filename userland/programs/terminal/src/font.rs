static FONT_DATA: &[u8] = include_bytes!("zap-ext-light16.psf");

const PSF1_MAGIC0: u8 = 0x36;
const PSF1_MAGIC1: u8 = 0x04;
const PSF1_MODE512: u8 = 0x01;
const PSF1_MODEHASTAB: u8 = 0x02;
const PSF1_SEPARATOR: u16 = 0xFFFF;
const PSF1_STARTSEQ: u16 = 0xFFFE;

pub fn glyph_width() -> usize { 8 }

pub fn glyph_height() -> usize { font_charsize() }

pub fn glyph_row(c: char, row: usize) -> u8 {
    let offset = glyph_offset_for_char(c);
    if row >= glyph_height() {
        return 0;
    }
    FONT_DATA.get(offset + row).copied().unwrap_or(0)
}

fn font_charsize() -> usize { FONT_DATA[3] as usize }

fn font_glyph_count() -> usize { if (FONT_DATA[2] & PSF1_MODE512) != 0 { 512 } else { 256 } }

fn font_has_unicode_table() -> bool { (FONT_DATA[2] & PSF1_MODEHASTAB) != 0 }

fn font_glyph_offset(index: usize) -> Option<usize> {
    if FONT_DATA.len() < 4 || FONT_DATA[0] != PSF1_MAGIC0 || FONT_DATA[1] != PSF1_MAGIC1 {
        return None;
    }

    if index >= font_glyph_count() {
        return None;
    }

    let charsize = font_charsize();
    let offset = 4 + index * charsize;
    if offset + charsize > FONT_DATA.len() {
        return None;
    }
    Some(offset)
}

fn unicode_glyph_index(c: char) -> Option<usize> {
    if !font_has_unicode_table() {
        return None;
    }

    let charsize = font_charsize();
    let glyph_count = font_glyph_count();
    let mut offset = 4 + glyph_count * charsize;

    for glyph in 0..glyph_count {
        loop {
            if offset + 2 > FONT_DATA.len() {
                return None;
            }
            let value = u16::from_le_bytes([FONT_DATA[offset], FONT_DATA[offset + 1]]);
            offset += 2;
            if value == PSF1_SEPARATOR {
                break;
            }
            if value == PSF1_STARTSEQ {
                loop {
                    if offset + 2 > FONT_DATA.len() {
                        return None;
                    }
                    let seq = u16::from_le_bytes([FONT_DATA[offset], FONT_DATA[offset + 1]]);
                    offset += 2;
                    if seq == PSF1_SEPARATOR {
                        break;
                    }
                }
                break;
            }
            if char::from_u32(value as u32) == Some(c) {
                return Some(glyph);
            }
        }
    }
    None
}

fn glyph_offset_for_char(c: char) -> usize {
    if let Some(index) = unicode_glyph_index(c) {
        if let Some(offset) = font_glyph_offset(index) {
            return offset;
        }
    }

    if c.is_ascii() {
        if let Some(offset) = font_glyph_offset(c as usize) {
            return offset;
        }
    }

    font_glyph_offset(b'?' as usize).unwrap_or(4)
}
