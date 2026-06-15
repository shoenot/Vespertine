use alloc::{boxed::Box, string::String, vec::Vec};
use vespertine_std::fs::PathBuf;

pub enum BaseNode {
    Cmd(CommandNode),
    Pipe(Box<BaseNode>, Box<BaseNode>),
}

pub enum CommandNode {
    Run { exec: String, args: Vec<String> },
    Echo { args: Vec<String> },
    ChangeDir { path: PathBuf },
    NoOp,
}

