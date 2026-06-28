use alloc::string::String;

#[derive(Debug, Clone)]
pub enum ConfigError {
    Invalid(String),
    Parse(String),
    NotFound(String),
}

impl ConfigError {
    pub fn invalid(message: String) -> Self { Self::Invalid(message) }

    pub fn parse(message: String) -> Self { Self::Parse(message) }

    pub fn not_found(message: String) -> Self { Self::NotFound(message) }
}
