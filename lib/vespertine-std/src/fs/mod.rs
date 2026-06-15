mod dir;
mod file;

pub use dir::*;
pub use file::*;
use vespertine_rt::syscall::{SysError, sys_invoke};
use vespertine_abi::{AccessRights, HandleID, Invocation, DirectoryOp};
use crate::env;

pub fn parse_parent_and_name(path: &str) -> (&str, &str) {
    if let Some(idx) = path.rfind('/') {
        let parent = &path[..idx];
        let name = &path[idx + 1..];
        let parent = if parent.is_empty() && path.starts_with('/') {
            "/"
        } else {
            parent
        };
        (parent, name)
    } else {
        ("", path)
    }
}

pub fn walk_path(path: &str, rights: AccessRights) -> Result<HandleID, SysError> {
    walk_path_from(path, env::root(), env::cwd(), rights)
}

pub fn walk_path_from(path: &str, root: HandleID, cwd: HandleID, rights: AccessRights) -> Result<HandleID, SysError> {
    let op = DirectoryOp::Resolve { start: cwd, path_ptr: path.as_ptr() as usize, path_len: path.len(), rights };
    let res = sys_invoke(root, &Invocation::Directory(op))?;
    Ok(HandleID(res))
}

