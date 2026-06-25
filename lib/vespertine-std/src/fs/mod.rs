mod dir;
mod file;

pub use dir::*;
pub use file::*;
use vespertine_abi::{
    AccessRights,
    DirectoryOp,
    FileStat,
    HandleID,
    Invocation,
};
use vespertine_rt::syscall::sys_invoke;

use crate::{
    Error,
    env,
};

pub fn resolve(path: &Path<'_>, rights: AccessRights) -> Result<HandleID, Error> {
    path.validate().map_err(Error::from)?;
    resolve_from(path, env::root(), env::cwd(), rights)
}

pub fn resolve_from(path: &Path<'_>, root: HandleID, cwd: HandleID, rights: AccessRights) -> Result<HandleID, Error> {
    path.validate().map_err(Error::from)?;
    let op = DirectoryOp::Resolve { start: cwd, path_ptr: path.as_str().as_ptr() as usize, path_len: path.as_str().len(), rights };
    sys_invoke(root, &Invocation::Directory(op)).map(HandleID).map_err(Error::from)
}

pub fn stat(path: &Path<'_>) -> Result<FileStat, Error> {
    let handle = resolve(path, AccessRights::new())?;
    let file = File::from_handle(handle);
    file.stat()
}

pub fn link_object(parent: HandleID, name: &str, object: HandleID) -> Result<(), Error> {
    let op = DirectoryOp::Link { name: name.as_ptr() as usize, name_len: name.len(), handle_id: object };
    sys_invoke(parent, &Invocation::Directory(op)).map(|_| ()).map_err(Error::from)
}
