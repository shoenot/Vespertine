#![no_std]
#![no_main]

mod launch;
mod lexer;
use launch::*;
use lexer::Tokenizer;

use core::iter::Peekable;

use alloc::{
    format,
    string::{String, ToString},
    vec::{IntoIter, Vec},
};
use vespertine_abi::ProcessInitPackage;
use vespertine_rt::{print, println, source::read_line};
use vespertine_std::term::get_term_cursor_position;

use crate::lexer::{ShellError, Token};

extern crate alloc;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] shell error: {:?}", e);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellPath {
    pub abs: bool,
    pub components: Vec<String>,
}

impl ShellPath {
    pub fn new(raw: &str) -> Self {
        let components = raw
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .map(|s| s.to_string())
            .collect();

        if raw.starts_with('/') {
            Self {
                abs: true,
                components,
            }
        } else {
            Self {
                abs: false,
                components,
            }
        }
    }

    pub fn normalize(&mut self) {
        let mut stack = Vec::new();

        for component in &self.components {
            match component.as_str() {
                "." | "" => {}
                ".." => {
                    if stack.last().is_some_and(|last: &String| last != "..") {
                        stack.pop();
                    } else if !self.abs {
                        stack.push(component.clone());
                    }
                }
                other => stack.push(other.to_string()),
            }
        }
        self.components = stack;
    }

    pub fn join(&self, rel: &ShellPath) -> Self {
        if rel.abs {
            let mut path = rel.clone();
            path.normalize();
            return path;
        }

        let mut new = self.components.clone();
        new.extend(rel.components.clone());

        let mut new_path = Self {
            abs: self.abs,
            components: new,
        };
        new_path.normalize();
        new_path
    }
}

impl ToString for ShellPath {
    fn to_string(&self) -> String {
        match (self.abs, self.components.is_empty()) {
            (true, true) => String::from("/"),
            (true, false) => format!("/{}", self.components.join("/")),
            (false, true) => String::from("."),
            (false, false) => self.components.join("/"),
        }
    }
}

pub struct ShellContext {
    cwd: ShellPath,
}

impl ShellContext {
    pub fn new() -> Self {
        Self {
            cwd: ShellPath::new("/"),
        }
    }

    pub fn cwd(&self) -> &ShellPath {
        &self.cwd
    }

    pub fn update_cwd(&mut self, path: ShellPath) {
        self.cwd = self.cwd.join(&path);
    }
}

pub enum BaseNode {
    Cmd(CommandNode),
}

pub enum CommandNode {
    Run { exec: String, args: Vec<String> },
    Echo { args: Vec<String> },
    ChangeDir { path: ShellPath },
    NoOp,
}

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

#[unsafe(no_mangle)]
fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), ShellError> {
    let mut cxt = ShellContext::new();

    loop {
        if let Ok((_row, col)) = get_term_cursor_position()
            && col > 0
        {
            // print newline if the last program's output didn't do it
            println!("");
        }

        print!("{} \x1b[35m>> \x1b[0m", cxt.cwd().to_string());
        let mut buf = [0u8; 128];
        let n = read_line(&mut buf);

        let line = str::from_utf8(&buf[..n])
            .unwrap_or("")
            .trim_end_matches('\n')
            .trim();

        let mut tokenizer = Tokenizer;
        let tokens = match tokenizer.tokenize_line(String::from(line)) {
            Ok(tokens) => tokens,
            Err(error) => {
                println!("[ERROR] {}", error);
                continue;
            }
        };

        let mut parser = Parser::new(tokens);
        let base = match parser.parse_line() {
            Ok(base) => base,
            Err(error) => {
                println!("[ERROR] {}", error);
                continue;
            }
        };

        match base {
            BaseNode::Cmd(cmd) => run_command(&mut cxt, cmd),
        }
    }
}

fn run_command(context: &mut ShellContext, cmd: CommandNode) {
    match cmd {
        CommandNode::Run { exec, args } => {
            launch_command(exec.as_str(), &args);
        },
        CommandNode::Echo { args } => {
            for arg in args {
                print!("{}", arg);
                println!("")
            }
        },
        CommandNode::ChangeDir { path } => {
            context.update_cwd(path);
        },
        CommandNode::NoOp => {},
    }
}
