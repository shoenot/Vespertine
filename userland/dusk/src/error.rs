use core::fmt::Display;

use alloc::string::String;
use vespertine_std::Error;

#[derive(Debug)]
pub enum ShellError {
    InvalidToken,
    ExpectedToken,
    NoCursorPosition,
    TerminalError,
}

impl Display for ShellError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellError::InvalidToken => write!(f, "DUSK: invalid token"),
            ShellError::ExpectedToken => write!(f, "DUSK: expected appropriate token"),
            ShellError::TerminalError => write!(f, "DUSK: terminal error"),
            ShellError::NoCursorPosition => write!(f, "DUSK: failure to get cursor position"),
        }
    }
}

