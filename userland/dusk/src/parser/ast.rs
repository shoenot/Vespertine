use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use vespertine_std::fs::PathBuf;

pub enum BaseNode {
    Cmd(CommandNode),
    Pipe(Box<BaseNode>, Box<BaseNode>),
}

pub enum CommandNode {
    Run { exec: String, args: Vec<String> },
    ChangeDir { path: PathBuf },
    ClearScreen,
    GetMetadata { app: String },
    NoOp,
}
