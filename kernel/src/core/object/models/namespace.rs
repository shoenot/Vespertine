use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{boxed::Box, vec};
use alloc::sync::Arc;
use async_trait::async_trait;
use vespertine_abi::{AccessRights, DirectoryOp, HandleID, Invocation};
use vespertine_common::path::{Component, Components};

use crate::arch::x86_64::task::syscall::safe_copy_from;
use crate::core::object::obj::ObjectType;
use crate::core::{object::{invoke::InvocationError, obj::KernelObject}, thread::get_current_process};

static NEXT_LOCATION_ID: AtomicUsize = AtomicUsize::new(1);

const PATH_MAX: usize = 4096;

enum Resolved {
    Directory(Arc<DirLocation>),
    Object(Arc<dyn KernelObject>, AccessRights),
}

#[derive(Debug, Clone)]
pub struct DirLocation {
    id: usize,
    dir: Arc<dyn KernelObject>,
    parent: Option<Arc<DirLocation>>,
}

impl DirLocation {
    pub fn root(dir: Arc<dyn KernelObject>) -> Arc<Self> {
        Arc::new(Self { 
            id: NEXT_LOCATION_ID.fetch_add(1, Ordering::Relaxed), 
            dir, parent: None,
        })
    }

    pub fn child(dir: Arc<dyn KernelObject>, parent: Arc<DirLocation>) -> Arc<Self> {
        Arc::new(Self { 
            id: NEXT_LOCATION_ID.fetch_add(1, Ordering::Relaxed), 
            dir, parent: Some(parent),
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn directory(&self) -> Arc<dyn KernelObject> {
        self.dir.clone()
    }

    pub fn parent(&self) -> Option<Arc<DirLocation>> {
        self.parent.clone()
    }

    pub fn is_beneath(&self, root_id: usize) -> bool {
        if self.id == root_id {
            return true;
        }

        let mut cursor = self.parent.clone();

        while let Some(location) = cursor {
            if location.id == root_id {
                return true;
            }
            cursor = location.parent.clone();
        }

        false
    }

    fn arc_clone(&self) -> Arc<Self> {
        Arc::new(self.clone())
    }

    async fn resolve(&self, start_handle: HandleID, path: &str, requested_rights: AccessRights, root_rights: AccessRights) -> Result<usize, InvocationError> {
        let root = self.arc_clone();
        let (start, start_rights) = resolve_location(start_handle, AccessRights::READ)?;
        let mut ancestry = if path.starts_with('/') {
            vec![(root.clone(), root_rights)]
        } else {
            ancestry_from(&root, &start, start_rights)?
        };
        let mut resolved = Resolved::Directory(ancestry.last().ok_or(InvocationError::InvalidArgument)?.0.clone());

        for component in Components::new(path) {
            match component {
                Component::Root => {
                    ancestry.truncate(1);
                    resolved = Resolved::Directory(ancestry[0].0.clone());
                },
                Component::Current => {
                    if !matches!(resolved, Resolved::Directory(_)) {
                        return Err(InvocationError::InvalidArgument);
                    }
                },
                Component::Parent => {
                    if !matches!(resolved, Resolved::Directory(_)) {
                        // "file/.." is invalid e.g.
                        return Err(InvocationError::InvalidArgument);
                    }
                    if ancestry.len() > 1 { ancestry.pop(); }

                    resolved = Resolved::Directory(ancestry.last().ok_or(InvocationError::InvalidArgument)?.0.clone());
                },
                Component::Normal(name) => {
                    if !matches!(resolved, Resolved::Directory(_)) {
                        return Err(InvocationError::InvalidArgument);
                    }

                    let (current, current_rights) = ancestry.last().ok_or(InvocationError::InvalidArgument)?;

                    let (child, child_rights) = lookup_raw_child(&current.directory(), name, *current_rights).await?;

                    if child.object_type() == ObjectType::Directory {
                        let child_location = DirLocation::child(child, current.clone());
                        ancestry.push((child_location.clone(), child_rights));
                        resolved = Resolved::Directory(child_location);
                    } else {
                        resolved = Resolved::Object(child, child_rights);
                    }
                }
            }
        }

        let (object, effective_rights): (Arc<dyn KernelObject>, AccessRights) = match resolved {
            Resolved::Directory(location) => {
                let rights = ancestry.last().ok_or(InvocationError::InvalidArgument)?.1;
                (location, rights)
            },
            Resolved::Object(object, rights) => {
                (object, rights)
            },
        };

        if !effective_rights.contains(requested_rights) {
            return Err(InvocationError::AccessDenied);
        }
        let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
        let handle = proc.proc_handles.write().insert(object, requested_rights);

        Ok(handle.0)
    }

    async fn lookup(&self, name_ptr: usize, name_len: usize, rights: AccessRights) -> Result<usize, InvocationError> {
        let mut bytes = vec![0u8; name_len];

        if !safe_copy_from(bytes.as_mut_ptr(), name_ptr as *const u8, name_len) {
            return Err(InvocationError::InvalidPointer);
        }

        let name = core::str::from_utf8(&bytes).map_err(|_| InvocationError::InvalidEncoding)?;

        match name {
            "." => register_result(self.arc_clone(), rights), 
            ".." => {
                Err(InvocationError::UnsupportedOperation)
            },
            _ => {
                let (child, child_rights) =
                    lookup_raw_child(&self.directory(), name, rights).await?;

                let result: Arc<dyn KernelObject> =
                    if child.object_type() == ObjectType::Directory {
                        wrap_child_directory(child, self.arc_clone())
                    } else {
                        child
                    };

                register_result(result, child_rights)
            }
        }
    }

    async fn create_dir(&self, invocation: Invocation, rights: AccessRights) -> Result<usize, InvocationError> {
        let temporary = HandleID(self.dir.invoke(invocation, rights).await?);
    
        let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
    
        let (child, child_rights) = {
            let table = proc.proc_handles.read();
            let entry = table.get(&temporary).ok_or(InvocationError::InvalidHandle)?;
            (entry.object.clone(), entry.rights)
        };
    
        proc.proc_handles.write().close(temporary)?;
    
        if child.object_type() != ObjectType::Directory { return Err(InvocationError::InvalidArgument); }
    
        let location = DirLocation::child(child, self.arc_clone());
        register_result(location, child_rights)
    }
}

#[async_trait]
impl KernelObject for DirLocation {
    fn type_name(&self) -> &'static str {
        "Directory"
    }

    fn object_type(&self) -> ObjectType { ObjectType::Directory }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    async fn invoke(&self, invocation: Invocation, rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Resolve { start, path_ptr, path_len, rights: requested_rights }) => {
                let path = copy_path(path_ptr, path_len)?;
                self.resolve(start, &path, requested_rights, rights).await
            },
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => {
                self.lookup(name, name_len, rights).await
            },
            op @ Invocation::Directory(DirectoryOp::CreateDir { .. }) => {
                self.create_dir(op, rights).await
            }
            other => self.dir.invoke(other, rights).await,
        }
    }


}

fn resolve_location(handle: HandleID, required_rights: AccessRights) -> Result<(Arc<DirLocation>, AccessRights), InvocationError> {
    let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
    let table = proc.proc_handles.read();
    let entry = table.resolve_entry(handle, required_rights)?;

    let location = entry.object.as_any().downcast_ref::<DirLocation>().ok_or(InvocationError::InvalidArgument)?;
    Ok((location.arc_clone(), entry.rights))
}

fn copy_path(path_ptr: usize, path_len: usize) -> Result<String, InvocationError> {
    if path_len > PATH_MAX {
        return Err(InvocationError::NameTooLong);
    }

    if path_len == 0 {
        return Ok(String::new());
    }

    let mut bytes = vec![0u8; path_len];

    if !safe_copy_from(bytes.as_mut_ptr(), path_ptr as *const u8, path_len) {
        return Err(InvocationError::InvalidPointer);
    }

    String::from_utf8(bytes).map_err(|_| InvocationError::InvalidEncoding)
}

async fn lookup_raw_child(dir: &Arc<dyn KernelObject>, name: &str, rights: AccessRights) -> Result<(Arc<dyn KernelObject>, AccessRights), InvocationError> {
    let op = DirectoryOp::Lookup { name: name.as_ptr() as usize, name_len: name.len() };
    let result = dir.invoke(Invocation::Directory(op), rights).await?;

    let temp = HandleID(result);
    let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
    let entry = {
        let table = proc.proc_handles.read();
        let entry = table.get(&temp).ok_or(InvocationError::InvalidHandle)?;
        let child_rights = entry.rights & rights;
        (entry.object.clone(), child_rights)
    };

    proc.proc_handles.write().close(temp)?;
    Ok(entry)
}

fn ancestry_from(root: &Arc<DirLocation>, start: &Arc<DirLocation>, rights: AccessRights) -> Result<Vec<(Arc<DirLocation>, AccessRights)>, InvocationError> {
    let mut reversed = Vec::new();
    let mut cursor = start.clone();

    loop {
        reversed.push(cursor.clone());
        if cursor.id() == root.id() {
            break;
        }
        cursor = cursor.parent().ok_or(InvocationError::AccessDenied)?;
    }

    reversed.reverse();
    Ok(reversed.into_iter().map(|location| (location, rights)).collect())
}

fn wrap_child_directory(child: Arc<dyn KernelObject>, parent: Arc<DirLocation>) -> Arc<DirLocation> {
    DirLocation::child(child, parent)
}

fn register_result(object: Arc<dyn KernelObject>, rights: AccessRights) -> Result<usize, InvocationError> {
    let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
    Ok(proc.proc_handles.write().insert(object, rights).0)
}
