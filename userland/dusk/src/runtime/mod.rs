pub mod env;
use alloc::string::{
    String,
    ToString,
};

use vespertine_abi::app::hesper::{HESPER_STATUS_INVALID_REQUEST, HESPER_STATUS_NOT_FOUND, HESPER_STATUS_OK};
use vespertine_rt::{
    print,
    println,
};
use vespertine_std::hesper::{
    AppMetadataResponse, Launcher, 
};
use vespertine_std::{
    Error,
    term,
};

use crate::error::ShellError;
use crate::lexer::Tokenizer;
use crate::parser::Parser;
use crate::parser::ast::{
    BaseNode,
    CommandNode,
};
use crate::runtime::env::ShellContext;
use crate::sys::{
    ShellResult,
    launch_base,
    launch_command,
};

pub struct ShellRuntime {
    pub context: ShellContext,
}

impl ShellRuntime {
    pub fn new() -> Self { Self { context: ShellContext::new() } }

    pub fn eval(&mut self, line: String) -> Result<ShellResult, ShellError> {
        let mut tokenizer = Tokenizer;
        let tokens = tokenizer.tokenize_line(line)?;
        let mut parser = Parser::new(tokens);
        let base = parser.parse_base()?;

        let res = match base {
            BaseNode::Cmd(cmd) => self.run_command(cmd),
            BaseNode::Pipe(..) => launch_base(base, &self.context),
        };

        self.context.last_result = res.clone();
        Ok(res)
    }

    pub fn draw_prompt(&self) {
        print!("{} \x1b[35m{} >> \x1b[0m", self.context.cwd().to_string(), self.context.status());
    }

    pub fn run_command(&mut self, cmd: CommandNode) -> ShellResult {
        match cmd {
            CommandNode::Run { exec, args } => {
                return launch_command(exec.as_str(), &args, &self.context);
            }
            CommandNode::ChangeDir { path } => {
                let display = path.to_string();
                match self.context.change_dir(path) {
                    Ok(_) => ShellResult::None,
                    Err(error) => ShellResult::ChangeDirFail(display, error),
                }
            }
            CommandNode::ClearScreen => match term::clear_term_screen() {
                Ok(_) => {
                    self.draw_prompt();
                    ShellResult::None
                }
                Err(error) => ShellResult::InternalError(error),
            },
            CommandNode::GetMetadata { app } => match get_app_metadata(app.as_str()) {
                Ok(md) => {
                    println!("id: {}; input: {}; output: {};", md.app_id, md.input as u8, md.output as u8);
                    ShellResult::None
                }
                Err(error) => ShellResult::InternalError(error),
            },
            CommandNode::NoOp => ShellResult::None,
        }
    }
}

pub fn get_app_metadata(name: &str) -> Result<AppMetadataResponse, Error> {
    let mut launcher = Launcher::connect()?;
    let response = launcher.metadata(name)?;

    if response.status != HESPER_STATUS_OK {
        return Err(match response.status {
            HESPER_STATUS_NOT_FOUND => {
                Error::not_found("application was not found".into())
            },
            HESPER_STATUS_INVALID_REQUEST => {
                Error::invalid_argument("application manifest is invalid".into())
            },
            _ => Error::unknown("Hesper returned an unknown status".into()),
        });
    }

    Ok(response)
}
