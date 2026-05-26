use vespertine_abi::{DirectoryOp, HandleID, Invocation};
use vespertine_rt::syscall::{SysError, sys_close, sys_invoke};


pub fn walk_path(path: &str, root: HandleID) -> Result<HandleID, SysError> {
    let mut current = root;
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        let op = Invocation::Directory(DirectoryOp::Lookup { name: segment.as_ptr(), name_len: segment.len() });
        let next = HandleID(sys_invoke(current, &op)?);
        if current != root { sys_close(current)?; }
        current = next;
    }
    Ok(current)
}

