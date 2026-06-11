use core::fmt::Display;

use alloc::{string::String, vec::Vec};

#[derive(Debug)]
pub enum ShellError {
    InvalidToken,
    ExpectedToken,
    NoCursorPosition,
}

impl Display for ShellError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellError::InvalidToken => write!(f, "Shell Error: invalid token"),
            ShellError::ExpectedToken => write!(f, "Shell Error: expected appropriate token"),
            ShellError::NoCursorPosition => {
                write!(f, "Shell Error: failure to get cursor position")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(String),

    Pipe,
    Redirect,
}

pub struct Tokenizer;

impl Tokenizer {
    fn split_line(&mut self, line: String) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    fn tokenize(&mut self, text: String) -> Result<Token, ShellError> {
        let tok = match text.as_str() {
            "|" => Token::Pipe,
            ">" => Token::Redirect,
            other => self.process_word(other)?,
        };
        Ok(tok)
    }

    fn process_word(&mut self, word: &str) -> Result<Token, ShellError> {
        let tok = Token::Word(String::from(word));
        Ok(tok)
    }

    pub fn tokenize_line(&mut self, line: String) -> Result<Vec<Token>, ShellError> {
        let words = self.split_line(line);
        let mut tokens = Vec::new();

        for word in words {
            tokens.push(self.tokenize(word)?);
        }

        Ok(tokens)
    }
}
