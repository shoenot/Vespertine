use alloc::{collections::vec_deque::VecDeque, vec::Vec};

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
        if self.cursor < self.buf.len() {
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
        let mut cursor = pos.max(self.buf.len());
        cursor = 0usize.min(cursor);
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
}

pub struct LineState {
    current: LineBuffer,
    history: VecDeque<LineBuffer>,
    history_pos: usize,
    current_draft: LineBuffer
}

impl LineState {
    fn new(old_history: Option<VecDeque<LineBuffer>>) -> Self {
        let history = if let Some(h) = old_history { h } else { VecDeque::new() };
        Self { current: LineBuffer::new(), history, history_pos: 0, current_draft: LineBuffer::new() }
    }

    fn history_up(&mut self) {
        if self.history.len() == 0 { return };
        if !self.current.is_empty() {
            self.current_draft = self.current.clone();
        }

        self.history_pos += 1;
        self.current = self.history[self.history_pos - 1].clone();
        self.current.set_cursor_end();
    }

    fn history_down(&mut self) {
        if self.history_pos == 0 { return };
        self.history_pos = self.history_pos.saturating_sub(1);
        // current_draft starts off empty
        if self.history_pos == 0 {
            self.current = self.current_draft.clone();
            self.current_draft = LineBuffer::new();
        } else {
            self.current = self.history[self.history_pos].clone();
            self.history_pos -= 1
        }
        self.current.set_cursor_end();
    }

    fn submit(&mut self) -> LineBuffer {
        let ret = self.current.clone();
        self.history.push_front(self.current.clone());
        self.current = LineBuffer::new();
        self.current_draft = LineBuffer::new();
        ret
    }
}
