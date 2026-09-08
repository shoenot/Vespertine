use vstd::prelude::*;

pub enum BaseNode {
    Cmd(CommandNode),
    Pipe(Box<BaseNode>, Box<BaseNode>),
}

pub enum CommandNode {
    Run { exec: String, args: Vec<String> },
    ChangeDir { path: PathBuf },
    ClearScreen,
    GetMetadata { app: String },
    Autopsy,
    NoOp,
}
