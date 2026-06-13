use alloc::{string::String, vec::Vec};

use crate::runtime::ShellPath;

pub enum BaseNode {
    Cmd(CommandNode),
}

pub enum CommandNode {
    Run { exec: String, args: Vec<String> },
    Echo { args: Vec<String> },
    ChangeDir { path: ShellPath },
    NoOp,
}

