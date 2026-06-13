use vespertine_abi::{AccessRights, DirectoryOp, HandleID, Invocation};
use vespertine_rt::syscall::{SysError, sys_close, sys_invoke};

use crate::env;

pub fn walk_path(path: &str, rights: AccessRights) -> Result<HandleID, SysError> {
    walk_path_from(path, env::root(), env::cwd(), rights)
}

pub fn walk_path_from(path: &str, root: HandleID, cwd: HandleID, rights: AccessRights) -> Result<HandleID, SysError> {
    let op = DirectoryOp::Resolve { start: cwd, path_ptr: path.as_ptr() as usize, path_len: path.len(), rights };
    let res = sys_invoke(root, &Invocation::Directory(op))?;
    Ok(HandleID(res))
}
