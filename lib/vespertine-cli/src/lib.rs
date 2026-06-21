#![no_std]
#![no_main]

extern crate alloc;

pub mod args;

use alloc::format;
use alloc::string::String;

use vespertine_std::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    UnknownOption(String),
    UnexpectedPositional(String),
    UnexpectedValue(String),
    MissingValue(String),
    MissingArgument(String),
    InvalidOption(String),
}

impl From<CliError> for Error {
    fn from(value: CliError) -> Self {
        match value {
            CliError::UnknownOption(s) => Error::invalid_argument(format!("Unknown option: \"{}\"", s)),
            CliError::UnexpectedPositional(s) => Error::invalid_argument(format!("Unexpected positional: \"{}\"", s)),
            CliError::UnexpectedValue(s) => Error::invalid_argument(format!("Unexpected value: \"{}\"", s)),
            CliError::MissingValue(s) => Error::invalid_argument(format!("Missing value: \"{}\"", s)),
            CliError::MissingArgument(s) => Error::invalid_argument(format!("Missing argument: \"{}\"", s)),
            CliError::InvalidOption(s) => Error::invalid_argument(format!("Invalid option: \"{}\"", s)),
        }
    }
}
