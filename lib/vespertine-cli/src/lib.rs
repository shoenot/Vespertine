#![no_std]
#![no_main]

extern crate alloc;

pub mod args;

use alloc::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    UnknownOption(String),
    UnexpectedPositional(String),
    UnexpectedValue(String),
    MissingValue(String),
    MissingArgument(String),
    InvalidOption(String),
}
