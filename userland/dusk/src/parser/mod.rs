pub mod ast;

use core::iter::Peekable;

use ast::*;

use alloc::{string::String, vec::{IntoIter, Vec}};

use crate::{error::ShellError, lexer::Token, runtime::ShellPath};


pub struct Parser {
    tokens: Peekable<IntoIter<Token>>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens: tokens.into_iter().peekable(),
        }
    }

    fn advance(&mut self) -> Option<Token> {
        self.tokens.next()
    }

    fn peek(&mut self) -> Option<&Token> {
        self.tokens.peek()
    }

    pub fn parse_command(&mut self) -> Result<CommandNode, ShellError> {
        let cmd = self.advance();
        if cmd.is_none() {
            return Ok(CommandNode::NoOp);
        };

        let node = match cmd.unwrap() {
            Token::Word(exec) => match exec.as_str() {
                "run" => {
                    let exec_tok = self.advance().ok_or(ShellError::ExpectedToken)?;
                    let Token::Word(exec) = exec_tok else {
                        return Err(ShellError::InvalidToken);
                    };
                    CommandNode::Run {
                        exec,
                        args: self.collect_args()?,
                    }
                }
                "echo" => CommandNode::Echo {
                    args: self.collect_args()?,
                },
                "cd" => match self.advance() {
                    None => CommandNode::ChangeDir {
                        path: ShellPath::new("/"),
                    },
                    Some(Token::Word(path)) => {
                        if self.peek().is_some() {
                            return Err(ShellError::InvalidToken);
                        }
                        CommandNode::ChangeDir {
                            path: ShellPath::new(path.as_str()),
                        }
                    }
                    _ => return Err(ShellError::InvalidToken),
                },
                _ => CommandNode::Run {
                    exec,
                    args: self.collect_args()?,
                },
            },
            _ => return Err(ShellError::InvalidToken),
        };

        Ok(node)
    }

    pub fn collect_args(&mut self) -> Result<Vec<String>, ShellError> {
        let mut args = Vec::new();
        while let Some(token) = self.advance() {
            match token {
                Token::Word(word) => args.push(word),
                _ => return Err(ShellError::InvalidToken),
            }
        }
        Ok(args)
    }

    pub fn parse_line(&mut self) -> Result<BaseNode, ShellError> {
        Ok(BaseNode::Cmd(self.parse_command()?))
    }
}
