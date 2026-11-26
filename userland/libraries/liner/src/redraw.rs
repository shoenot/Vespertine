use core::fmt::Write as _;

use alloc::{string::String, vec::Vec};
use alloc::vec;
use vstd::Error;
use vstd::Write;

struct Frame {
    rows: Vec<String>,
    caret: (usize, usize),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Colors {
    fg: Option<u8>,
    bg: Option<u8>,
}

fn take_sgr(input: &str, mut colors: Colors) -> Option<(Colors, &str)> {
    let tail = input.strip_prefix("\x1b[")?;
    let len = tail.bytes().take_while(|b| b.is_ascii_digit() || *b == b';').count();
    let rest = tail[len..].strip_prefix('m')?;

    for param in tail[..len].split(';') {
        let code = if param.is_empty() { 0 } else { param.parse::<u8>().ok()? };
        match code {
            0 => colors = Colors::default(),
            30..=37 => colors.fg = Some(code),
            40..=47 => colors.bg = Some(code),
            _ => return None,
        }
    }

    Some((colors, rest))
}

fn prompt_chars(mut rest: &str) -> impl Iterator<Item = (char, Colors)> + '_ {
    let mut colors = Colors::default();

    core::iter::from_fn(move || {
        while let Some((next, tail)) = take_sgr(rest, colors) {
            colors = next;
            rest = tail;
        }

        let ch = rest.chars().next()?;
        rest = &rest[ch.len_utf8()..];
        Some((ch, colors))
    })
}

fn layout(prompt: &str, text: &[char], cursor: usize, width: usize) -> Frame {
    let mut rows = vec![String::new()];
    let mut col = 0;
    let mut caret = (0, 0);
    let mut row_colors = Colors::default();
    let prompt_len = prompt_chars(prompt).count();
    let glyphs = prompt_chars(prompt)
        .chain(text.iter().copied().map(|ch| (ch, Colors::default())));

    for (i, (ch, colors)) in glyphs.enumerate() {
        if ch != '\n' && col == width {
            rows.push(String::new());
            col = 0;
            row_colors = Colors::default();
        }

        if i == prompt_len + cursor {
            caret = (rows.len() - 1, col);
        }

        if ch == '\n' {
            rows.push(String::new());
            col = 0;
            row_colors = Colors::default();
        } else {
            let row = rows.last_mut().unwrap();
            if colors != row_colors {
                row.push_str("\x1b[0m");
                if let Some(fg) = colors.fg {
                    write!(row, "\x1b[{fg}m").unwrap();
                }
                if let Some(bg) = colors.bg {
                    write!(row, "\x1b[{bg}m").unwrap();
                }
                row_colors = colors;
            }
            row.push(if ch.is_control() { '�' } else { ch });
            col += 1;
        }
    }

    if cursor == text.len() {
        caret = (rows.len() - 1, col);
    }

    Frame { rows, caret }
}

#[derive(Default)]
pub struct Redraw {
    painted_rows: usize,
    caret_row: usize,
}

impl Redraw {
    pub fn paint(
        &mut self,
        out: &impl Write,
        prompt: &str,
        text: &[char],
        cursor: usize,
        rows: usize,
        cols: usize,
    ) -> Result<(), Error> {
        if rows == 0 || cols < 2 || cursor > text.len() {
            return Err(Error::invalid_argument("invalid editor geometry".into()));
        }

        let frame = layout(prompt, text, cursor, cols - 1);
        let (row, col) = frame.caret;

        // for drafts taller than the screen, keep the caret in view.
        let first = (row + 1).saturating_sub(rows);
        let last = (first + rows).min(frame.rows.len());
        let visible = &frame.rows[first..last];
        let caret_row = row - first;
        let count = self.painted_rows.min(rows).max(visible.len());

        let mut bytes = String::from("\x1b[0m\r");
        if self.caret_row != 0 {
            write!(bytes, "\x1b[{}A", self.caret_row).unwrap();
        }

        for i in 0..count {
            if i != 0 {
                bytes.push_str("\r\n");
            }
            bytes.push_str("\x1b[2K");
            if let Some(line) = visible.get(i) {
                bytes.push_str(line);
            }
        }

        bytes.push_str("\x1b[0m");

        // we are on the last painted row. move back to the new caret.
        let up = count - 1 - caret_row;
        bytes.push('\r');
        if up != 0 {
            write!(bytes, "\x1b[{up}A").unwrap();
        }
        if col != 0 {
            write!(bytes, "\x1b[{col}C").unwrap();
        }

        out.write_all(bytes.as_bytes())?;
        self.painted_rows = visible.len();
        self.caret_row = caret_row;
        Ok(())
    }
}
