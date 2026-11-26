pub enum InputType {
    Move(Movement),
    Backspace,
    Delete,
    ClearLine,
    Return,
    Insert(char),
}

pub enum Movement {
    Up,
    Down,
    Left,
    Right,
}

pub enum Modifiers {
    Ctrl,
    Alt,
    Shift,
    Super,
}

pub struct ByteDecoder {
    state: State
}

enum State {
    Ground,
    Escape,
    Ss3,
    Csi { params: [u8; 8], len: usize },
    DiscardCsi,
    Utf8 { bytes: [u8; 4], len: usize },
}

impl ByteDecoder {
    pub fn new() -> Self {
        Self { state: State::Ground }
    }

    pub fn push(&mut self, byte: u8) -> Option<InputType> {
        match core::mem::replace(&mut self.state, State::Ground) {
            State::Ground => self.ground(byte),
            // Terminal sends F1–F4 as ESC O followed by one byte.
            State::Escape => {
                self.state = match byte {
                    b'[' => State::Csi { params: [0; 8], len: 0 },
                    b'O' => State::Ss3,
                    0x1b => State::Escape,
                    b'\r' | b'\n' => return self.ground(byte),
                    _ => State::Ground,
                };
                None
            }
            State::Ss3 => None,
            State::Csi { mut params, len } => match byte {
                0x40..=0x7e => Self::csi_action(&params[..len], byte),
                0x20..=0x3f => {
                    if len == params.len() {
                        self.state = State::DiscardCsi;
                    } else {
                        params[len] = byte;
                        self.state = State::Csi { params, len: len + 1 };
                    }
                    None
                }
                _ => self.ground(byte),
            },
            State::DiscardCsi => match byte {
                0x40..=0x7e => None,
                0x20..=0x3f => {
                    self.state = State::DiscardCsi;
                    None
                }
                _ => self.ground(byte),
            },
            State::Utf8 { mut bytes, len } => {
                if !(0x80..=0xbf).contains(&byte) {
                    return self.ground(byte);
                }

                bytes[len] = byte;
                let len = len + 1;
                match core::str::from_utf8(&bytes[..len]) {
                    Ok(s) => s.chars().next().map(InputType::Insert),
                    Err(err) if err.error_len().is_none() && len < bytes.len() => {
                        self.state = State::Utf8 { bytes, len };
                        None
                    }
                    Err(_) => None,
                }
            }
        }
    }

    fn ground(&mut self, byte: u8) -> Option<InputType> {
        match byte {
            b'\r' | b'\n' => Some(InputType::Return),
            0x08 | 0x7f => Some(InputType::Backspace),
            0x15 => Some(InputType::ClearLine), // Ctrl+U
            0x03 => Some(InputType::ClearLine),
            0x20..=0x7e => Some(InputType::Insert(char::from(byte))),
            0x1b => {
                self.state = State::Escape;
                None
            }
            0xc2..=0xf4 => {
                self.state = State::Utf8 { bytes: [byte, 0, 0, 0], len: 1 };
                None
            }
            _ => None,
        }
    }

    fn csi_action(params: &[u8], final_byte: u8) -> Option<InputType> {
        use Movement::{Down, Left, Right, Up};

        match (params, final_byte) {
            (b"", b'A') => Some(InputType::Move(Up)),
            (b"", b'B') => Some(InputType::Move(Down)),
            (b"", b'C') => Some(InputType::Move(Right)),
            (b"", b'D') => Some(InputType::Move(Left)),
            (b"3", b'~') => Some(InputType::Delete),
            (b"13;2", b'u') => Some(InputType::Insert('\n')),
            _ => None,
        }
    }
}
