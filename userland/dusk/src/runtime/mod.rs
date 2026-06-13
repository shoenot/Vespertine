mod path;
pub mod env;
use alloc::string::{String, ToString};
pub use path::*;
use vespertine_abi::ProcessExitInfo;
use vespertine_rt::{print, println};

use crate::{error::ShellError, lexer::Tokenizer, parser::{Parser, ast::{BaseNode, CommandNode}}, runtime::env::ShellContext, sys::{ShellResult, launch_command}};

pub struct ShellRuntime {
    pub context: ShellContext,
}

impl ShellRuntime {
    pub fn new() -> Self {
        Self { 
            context: ShellContext::new(),
        }
    }

    pub fn eval(&mut self, line: String) -> Result<ShellResult, ShellError> {
        let mut tokenizer = Tokenizer;
        let tokens = tokenizer.tokenize_line(line)?;
        let mut parser = Parser::new(tokens);
        let base = parser.parse_line()?;

        let res = match base {
            BaseNode::Cmd(cmd) => self.run_command(cmd),
        };

        self.context.last_result = res.clone();
        Ok(res)
    }

    pub fn run_command(&mut self, cmd: CommandNode) -> ShellResult {
        match cmd {
            CommandNode::Run { exec, args } => {
                return launch_command(exec.as_str(), &args, &self.context);
            },
            CommandNode::Echo { args } => {
                for arg in args {
                    print!("{}", arg);
                    println!("")
                }
                ShellResult::None
            },
            CommandNode::ChangeDir { path } => {
                let display = path.to_string();
                match self.context.change_dir(path) {
                    Ok(_) => ShellResult::None,
                    Err(error) => ShellResult::ChangeDirFail(display, error)
                }
            },
            CommandNode::NoOp => ShellResult::None,
        }
    }
}
