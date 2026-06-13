mod path;
pub mod env;
use alloc::string::String;
pub use path::*;
use vespertine_abi::ProcessExitInfo;
use vespertine_rt::{print, println};

use crate::{error::ShellError, lexer::Tokenizer, parser::{Parser, ast::{BaseNode, CommandNode}}, runtime::env::ShellContext, sys::launch_command};

pub struct ShellRuntime {
    pub context: ShellContext,
}

impl ShellRuntime {
    pub fn new() -> Self {
        Self { 
            context: ShellContext::new(),
        }
    }

    pub fn eval(&self, line: String) -> Result<(), ShellError> {
        let mut tokenizer = Tokenizer;
        let tokens = tokenizer.tokenize_line(line)?;
        let mut parser = Parser::new(tokens);
        let base = parser.parse_line()?;

        match base {
            BaseNode::Cmd(cmd) => self.run_command(cmd)?,
        }
    }

    pub fn run_command(&mut self, cmd: CommandNode) -> Result<Option<ProcessExitInfo>, ShellError> {
        match cmd {
            CommandNode::Run { exec, args } => {
                Ok(launch_command(exec.as_str(), &args, &self.context));
            },
            CommandNode::Echo { args } => {
                for arg in args {
                    print!("{}", arg);
                    println!("")
                }
            },
            CommandNode::ChangeDir { path } => {
                self.context.update_cwd(path);
            },
            CommandNode::NoOp => {},
        }
    }
}
