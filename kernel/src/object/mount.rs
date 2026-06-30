use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::object::invoke::InvocationError;
use crate::object::obj::KernelObject;
use crate::sync::RwLock;

#[derive(Debug)]
struct Mount {
    covered: Arc<dyn KernelObject>,
    root: Arc<dyn KernelObject>,
}

static MOUNTS: RwLock<Vec<Mount>> = RwLock::new(Vec::new());

pub fn mount(covered: Arc<dyn KernelObject>, root: Arc<dyn KernelObject>) -> Result<(), InvocationError> {
    if covered.as_directory().is_none() || root.as_directory().is_none() {
        return Err(InvocationError::InvalidArgument);
    }
    let mut mounts = MOUNTS.write();
    if mounts.iter().any(|mount| Arc::ptr_eq(&mount.covered, &covered)) {
        return Err(InvocationError::InvalidArgument);
    }
    mounts.push(Mount { covered, root });
    Ok(())
}

pub fn follow_mount(object: Arc<dyn KernelObject>) -> Arc<dyn KernelObject> {
    let mut current = object;
    loop {
        let mounted = {
            let mounts = MOUNTS.read();
            mounts.iter().find(|mount| Arc::ptr_eq(&mount.covered, &current)).map(|mount| mount.root.clone())
        };
        match mounted {
            Some(root) => current = root,
            None => return current,
        }
    }
}
