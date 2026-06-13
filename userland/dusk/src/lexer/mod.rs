use alloc::{string::String, vec::Vec};

use crate::error::ShellError;

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
