mod parse;
use alloc::{format, string::{String, ToString}, vec::Vec};
use parse::*;

use crate::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueMode {
    None,
    Required,
}

#[derive(Debug, Clone)]
pub struct Opt<'a> {
    pub id: &'a str,
    pub short: Option<char>,
    pub long: Option<&'a str>,
    pub value: ValueMode,
}

impl<'a> Opt<'a> {
    pub const fn flag(id: &'a str, short: Option<char>, long: Option<&'a str>) -> Self {
        Self { id, short, long, value: ValueMode::None }
    }

    pub const fn value(id: &'a str, short: Option<char>, long: Option<&'a str>) -> Self {
        Self { id, short, long, value: ValueMode::Required }
    }
}

#[derive(Debug, Clone)]
pub struct Command<'a> {
    name: &'a str,
    options: &'a [Opt<'a>],
    allow_positionals: bool,
}

impl<'a> Command<'a> {
    pub const fn new(name: &'a str) -> Self {
        Self { name, options: &[], allow_positionals: true }
    }

    pub const fn options(mut self, options: &'a [Opt<'a>]) -> Self {
        self.options = options;
        self
    }

    pub const fn allow_positionals(mut self, yes: bool) -> Self {
        self.allow_positionals = yes;
        self
    }

    fn find_short(&self, ch: char) -> Option<&Opt<'a>> {
        self.options.iter().find(|opt| opt.short == Some(ch))
    }

    fn find_long(&self, name: &str) -> Option<&Opt<'a>> {
        self.options.iter().find(|opt| opt.long == Some(name))
    }

    pub fn parse(&self, argv: &'a [String]) -> Result<Matches<'a>, CliError> {
        let mut out = Matches { command: self.name, options: Vec::new(), positionals: Vec::new() };

        let mut args = Arguments::new(argv);

        while let Some(arg) = args.next_arg()? {
            match arg {
                Arg::Positional(value) => {
                    if !self.allow_positionals {
                        return Err(CliError::UnexpectedPositional(value.to_string()));
                    }

                    out.positionals.push(value);
                },
                Arg::Short(ch) => {
                    let opt = self.find_short(ch).ok_or(CliError::UnknownOption(ch.to_string()))?;
                    match opt.value {
                        ValueMode::None => {
                            out.options.push(ParsedOpt { id: opt.id, value: None });
                        },
                        ValueMode::Required => {
                            let value = args.next_value(&format!("-{}", ch))?;
                            out.options.push(ParsedOpt { id: opt.id, value: Some(value) });
                        },
                    }
                },
                Arg::Long(name) => {
                    let opt = self.find_long(name).ok_or_else(|| CliError::UnknownOption(name.to_string()))?;

                    match opt.value {
                        ValueMode::None => {
                            out.options.push(ParsedOpt { id: opt.id, value: None });
                        },
                        ValueMode::Required => {
                            let value = args.next_value(&format!("--{}", name))?;
                            out.options.push(ParsedOpt { id: opt.id, value: Some(value) });
                        },
                    }
                },
                Arg::LongValue(name, value) => {
                    let opt = self
                        .find_long(name)
                        .ok_or_else(|| CliError::UnknownOption(name.to_string()))?;

                    match opt.value {
                        ValueMode::None => {
                            return Err(CliError::UnexpectedValue(format!("--{}", name)));
                        }

                        ValueMode::Required => {
                            out.options.push(ParsedOpt {
                                id: opt.id,
                                value: Some(value),
                            });
                        }
                    }
                }
            }
        }

        Ok(out)
    }
}

#[derive(Debug, Clone)]
pub struct ParsedOpt<'a> {
    pub id: &'a str,
    pub value: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Matches<'a> {
    command: &'a str,
    options: Vec<ParsedOpt<'a>>,
    positionals: Vec<&'a str>,
}

impl<'a> Matches<'a> {
    pub fn flag(&self, id: &str) -> bool {
        self.options.iter().any(|opt| opt.id == id)
    }

    pub fn value(&self, id: &str) -> Option<&'a str> {
        self.options.iter().find(|opt| opt.id == id).and_then(|opt| opt.value)
    }

    pub fn values(&'a self, id: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.options.iter().filter(move |opt| opt.id == id).filter_map(|opt| opt.value)
    }

    pub fn positionals(&self) -> &[&'a str] {
        &self.positionals
    }

    pub fn positional(&self, index: usize) -> Option<&'a str> {
        self.positionals.get(index).copied()
    }

    pub fn require_positional(&self, index: usize, name: &str) -> Result<&'a str, CliError> {
        self.positional(index)
            .ok_or_else(|| CliError::MissingArgument(name.to_string()))
    }

    pub fn positional_count(&self) -> usize {
        self.positionals.len()
    }

    pub fn has_positionals(&self) -> bool {
        !self.positionals.is_empty()
    }
}
