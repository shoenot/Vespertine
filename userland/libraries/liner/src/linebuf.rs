use alloc::{collections::vec_deque::VecDeque, string::String, vec::Vec};
use vstd::{Error, Read};

use crate::parser::{ByteDecoder, InputType, Movement};

pub const MAX_HISTORY: usize = 1000;

#[derive(Debug, Clone)]
pub struct LineBuffer {
    buf: Vec<char>,
    cursor: usize,
}

impl LineBuffer {
    const fn new() -> Self { LineBuffer { buf: Vec::new(), cursor: 0 } }

    fn insert(&mut self, ch: char) {
        self.buf.insert(self.cursor, ch);
        self.cursor += 1;
    }

    fn move_right(&mut self) {
        if self.cursor < self.buf.len() { self.cursor += 1 };
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buf.remove(self.cursor);
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
        }
    }

    fn len(&self) -> usize {
        self.buf.len()
    }

    fn is_empty(&self) -> bool {
        self.buf.len() == 0
    }

    fn set_cursor_absolute(&mut self, pos: usize) {
        let mut cursor = pos.min(self.buf.len());
        cursor = 0usize.max(cursor);
        self.cursor = cursor;
    }

    fn set_cursor_end(&mut self) {
        self.cursor = self.cursor.max(self.buf.len())
    }

    fn is_cursor_at_end(&self) -> bool {
        self.cursor == self.buf.len()
    }

    fn cursor_pos(&self) -> usize {
        self.cursor
    }

    fn get_buffer(&self) -> Vec<char> {
        self.buf.clone()
    }

    fn set_buffer(&mut self, buf: &Vec<char>) {
        self.buf = buf.clone();
        self.cursor = self.buf.len();
    }
}

pub struct LineState {
    current: LineBuffer,
    history: VecDeque<Vec<char>>,
    history_pos: usize,
    current_draft: LineBuffer
}

impl LineState {
    pub fn new(old_history: Option<VecDeque<Vec<char>>>) -> Self {
        let mut history = if let Some(h) = old_history { h } else { VecDeque::new() };
        if history.len() > MAX_HISTORY {
            history.truncate(MAX_HISTORY);
        }
        Self { current: LineBuffer::new(), history, history_pos: 0, current_draft: LineBuffer::new() }
    }

    fn history_up(&mut self) {
        if self.history.len() == 0 { return };
        if !self.current.is_empty() && self.history_pos == 0 {
            self.current_draft = self.current.clone();
        }

        if self.history_pos < self.history.len() {
            self.history_pos += 1;
            self.current.set_buffer(&self.history[self.history_pos - 1]);
        }
    }

    fn history_down(&mut self) {
        if self.history_pos == 0 { return };
        self.history_pos = self.history_pos.saturating_sub(1);
        // current_draft starts off empty
        if self.history_pos == 0 {
            self.current = self.current_draft.clone();
            self.current_draft = LineBuffer::new();
        } else {
            self.current.set_buffer(&self.history[self.history_pos - 1]);
        }
    }

    fn submit(&mut self) -> LineBuffer {
        let ret = self.current.clone();
        if !self.current.is_empty() {
            self.history.push_front(self.current.get_buffer());
            if self.history.len() > MAX_HISTORY {
                let _ = self.history.pop_back();
            }
        }
        self.history_pos = 0;
        self.current = LineBuffer::new();
        self.current_draft = LineBuffer::new();
        ret
    }

    fn consume(&mut self, input: InputType) -> Option<LineBuffer> {
        match input {
            InputType::Insert(c) => self.current.insert(c),
            InputType::Backspace => self.current.backspace(),
            InputType::Delete => self.current.delete(),
            InputType::ClearLine => self.current = LineBuffer::new(),
            InputType::Return => return Some(self.submit()),
            InputType::Move(m) => match m {
                Movement::Up => self.history_up(),
                Movement::Down => self.history_down(),
                Movement::Left => self.current.move_left(),
                Movement::Right => self.current.move_right(),
            }
        }
        None
    }

    pub fn read_line(
        &mut self, 
        input: &impl Read, 
        mut redraw: impl FnMut(&[char], usize) -> Result<(), Error>
    ) -> Result<Option<String>, Error> {
        let mut decoder = ByteDecoder::new();
        let mut byte = [0u8; 1];

        loop {
            if input.read(&mut byte)? == 0 {
                return Ok(None);
            }

            let Some(action) = decoder.push(byte[0]) else {
                continue;
            };

            if let Some(line) = self.consume(action) {
                // put the caret after the whole command before dusk prints the newline
                // even if Enter was pressed mid buffer
                redraw(&line.buf, line.buf.len())?;
                return Ok(Some(line.buf.into_iter().collect()));
            }

            redraw(&self.current.buf, self.current.cursor)?;
        }
    }
}
