use alloc::string::{String, ToString};
use alloc::vec::{IntoIter, Vec};

use crate::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg<'a> {
    Positional(&'a str),
    Short(char),
    Long(&'a str),
    LongValue(&'a str, &'a str),
}

pub struct Arguments<'a> {
    args: &'a [String],
    index: usize,
    end_options: bool,
    short_chars: Option<IntoIter<char>>,
}

impl<'a> Arguments<'a> {
    pub fn new(args: &'a [String]) -> Self {
        Self {
            args,
            index: 0,
            end_options: false,
            short_chars: None,
        }
    }

    pub fn next_arg(&mut self) -> Result<Option<Arg<'a>>, CliError> {
        if let Some(chars) = &mut self.short_chars {
            if let Some(ch) = chars.next() {
                return Ok(Some(Arg::Short(ch)));
            }
            self.short_chars = None;
        }

        if self.index >= self.args.len() {
            return Ok(None);
        }

        let raw = self.args[self.index].as_str();
        self.index += 1;

        if self.end_options {
            return Ok(Some(Arg::Positional(raw)));
        }

        if raw == "--" {
            self.end_options = true;
            return self.next_arg();
        }

        if raw == "-" {
            return Ok(Some(Arg::Positional(raw)));
        }

        if let Some(rest) = raw.strip_prefix("--") {
            if rest.is_empty() {
                return Err(CliError::InvalidOption(raw.to_string()));
            }

            if let Some(eq) = rest.find('=') {
                let name = &rest[..eq];
                let value = &rest[eq + 1..];

                if name.is_empty() {
                    return Err(CliError::InvalidOption(raw.to_string()));
                };

                return Ok(Some(Arg::LongValue(name, value)));
            }

            return Ok(Some(Arg::Long(rest)));
        }

        if let Some(rest) = raw.strip_prefix('-') {
            if rest.is_empty() {
                return Ok(Some(Arg::Positional(raw)));
            }

            let mut chars = rest.chars().collect::<Vec<_>>().into_iter();
            let first = chars.next().unwrap();

            self.short_chars = Some(chars);
            return Ok(Some(Arg::Short(first)));
        }

        Ok(Some(Arg::Positional(raw)))
    }

    pub fn next_value(&mut self, option: &str) -> Result<&'a str, CliError> {
        if self.short_chars.is_some() {
            self.short_chars = None;
        }

        if self.index >= self.args.len() {
            return Err(CliError::MissingValue(option.to_string()));
        }

        let value = self.args[self.index].as_str();
        self.index += 1;

        Ok(value)
    }
}
