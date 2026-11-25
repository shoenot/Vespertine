#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info = 0,
    Warn = 1,
    Error = 2,
    Debug = 3,
}

impl LogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }

    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => LogLevel::Info,
            1 => LogLevel::Warn,
            2 => LogLevel::Error,
            3 => LogLevel::Debug,
            _ => LogLevel::Info,
        }
    }
}
