use core::fmt::Display;

use alloc::string::String;
use vespertine_std::Error;

#[derive(Debug)]
pub enum ShellError {
    InvalidToken,
    ExpectedToken,
    NoCursorPosition,
    TerminalError,
    NotFound(String),
    AccessDenied(String),
    LaunchError(String, Error),
}

impl Display for ShellError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellError::InvalidToken => write!(f, "DUSK: invalid token"),
            ShellError::ExpectedToken => write!(f, "DUSK: expected appropriate token"),
            ShellError::TerminalError => write!(f, "DUSK: terminal error"),
            ShellError::NoCursorPosition => write!(f, "DUSK: failure to get cursor position"),
            ShellError::NotFound(n) => write!(f, "DUSK: not found: {}", n),
            ShellError::AccessDenied(n) => write!(f, "DUSK: access denied: {}", n),
            ShellError::LaunchError(n, e) => write!(f, "DUSK: {} launch error: {:?}", n, e),
        }
    }
}

