use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    DirectoryOp,
    HandleID,
    Invocation, ObjectType,
};
use vespertine_common::path::{
    Component,
    Components,
};

use crate::arch::x86_64::task::syscall::safe_copy_from;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::core::security::permissions::{
    FilePermissions,
    allowed_rights,
};
use crate::core::thread::get_current_process;

static NEXT_LOCATION_ID: AtomicUsize = AtomicUsize::new(1);

const PATH_MAX: usize = 4096;

#[derive(Debug)]
pub struct KernelNamespaceAuthority {
    _private: (),
}

pub(crate) fn kernel_namespace_authority() -> KernelNamespaceAuthority { KernelNamespaceAuthority { _private: () } }

enum KernelResolved {
    Directory(Arc<DirLocation>),
    Object(Arc<dyn KernelObject>),
}

enum Resolved {
    Directory(Arc<DirLocation>),
    Object(Arc<dyn KernelObject>),
}

#[derive(Debug, Clone)]
pub struct DirLocation {
    id: usize,
    dir: Arc<dyn KernelObject>,
    parent: Option<Arc<DirLocation>>,
}

impl DirLocation {
    pub fn root(dir: Arc<dyn KernelObject>) -> Arc<Self> {
        Arc::new(Self { id: NEXT_LOCATION_ID.fetch_add(1, Ordering::Relaxed), dir, parent: None })
    }

    pub fn child(dir: Arc<dyn KernelObject>, parent: Arc<DirLocation>) -> Arc<Self> {
        Arc::new(Self { id: NEXT_LOCATION_ID.fetch_add(1, Ordering::Relaxed), dir, parent: Some(parent) })
    }

    pub fn id(&self) -> usize { self.id }

    pub fn directory(&self) -> Arc<dyn KernelObject> { self.dir.clone() }

    pub fn parent(&self) -> Option<Arc<DirLocation>> { self.parent.clone() }

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

    pub(crate) fn arc_clone(&self) -> Arc<Self> { Arc::new(self.clone()) }

    async fn resolve(
        &self, start_handle: HandleID, path: &str, requested_rights: AccessRights, root_rights: AccessRights,
    ) -> Result<usize, InvocationError> {
        let root = self.arc_clone();
        let ns_ceiling = root_rights;
        let mut ancestry = if path.starts_with('/') {
            vec![root.clone()]
        } else {
            let (start, _) = resolve_location(start_handle, AccessRights::TRAVERSE)?;
            ancestry_from(&root, &start)?
        };
        let mut resolved = Resolved::Directory(ancestry.last().ok_or(InvocationError::InvalidArgument)?.clone());

        for component in Components::new(path) {
            match component {
                Component::Root => {
                    ancestry.truncate(1);
                    resolved = Resolved::Directory(ancestry[0].clone());
                }
                Component::Current => {
                    if !matches!(resolved, Resolved::Directory(_)) {
                        return Err(InvocationError::InvalidArgument);
                    }
                }
                Component::Parent => {
                    if !matches!(resolved, Resolved::Directory(_)) {
                        // "file/.." is invalid e.g.
                        return Err(InvocationError::InvalidArgument);
                    }
                    if ancestry.len() > 1 {
                        ancestry.pop();
                    }

                    resolved = Resolved::Directory(ancestry.last().ok_or(InvocationError::InvalidArgument)?.clone());
                }
                Component::Normal(name) => {
                    if !matches!(resolved, Resolved::Directory(_)) {
                        return Err(InvocationError::InvalidArgument);
                    }

                    let current = ancestry.last().ok_or(InvocationError::InvalidArgument)?;
                    let current_object: Arc<dyn KernelObject> = current.clone();
                    let current_user_rights = allowed_rights(&current_object)?;
                    let traversal_rights = ns_ceiling & current_user_rights;

                    if !traversal_rights.contains(AccessRights::TRAVERSE) {
                        return Err(InvocationError::AccessDenied);
                    }

                    let directory = current.directory();
                    let child = directory.as_directory().ok_or(InvocationError::InvalidArgument)?.lookup_child(name).await?;

                    if child.object_type() == ObjectType::Directory {
                        let child_location = DirLocation::child(child, current.clone());
                        ancestry.push(child_location.clone());
                        resolved = Resolved::Directory(child_location);
                    } else {
                        resolved = Resolved::Object(child);
                    }
                }
            }
        }

        let object: Arc<dyn KernelObject> = match resolved {
            Resolved::Directory(location) => location,
            Resolved::Object(object) => object,
        };

        let user_rights = allowed_rights(&object)?;
        let effective_rights = ns_ceiling & user_rights;

        if !effective_rights.contains(requested_rights) {
            return Err(InvocationError::AccessDenied);
        }

        register_result(object, requested_rights)
    }

    async fn lookup(&self, name_ptr: usize, name_len: usize, capability_rights: AccessRights) -> Result<usize, InvocationError> {
        let mut bytes = vec![0u8; name_len];

        if !safe_copy_from(bytes.as_mut_ptr(), name_ptr as *const u8, name_len) {
            return Err(InvocationError::InvalidPointer);
        }

        let name = core::str::from_utf8(&bytes).map_err(|_| InvocationError::InvalidEncoding)?;

        let self_object: Arc<dyn KernelObject> = self.arc_clone();
        let parent_user_rights = allowed_rights(&self_object)?;
        let parent_effective_rights = capability_rights & parent_user_rights;

        if !parent_effective_rights.contains(AccessRights::TRAVERSE) {
            return Err(InvocationError::AccessDenied);
        }

        match name {
            "." => register_result(self.arc_clone(), parent_effective_rights),
            ".." => Err(InvocationError::UnsupportedOperation),
            _ => {
                let directory = self.directory();
                let child = directory.as_directory().ok_or(InvocationError::InvalidArgument)?.lookup_child(name).await?;

                let child_user_rights = allowed_rights(&child)?;
                let child_effective_rights = capability_rights & child_user_rights;

                let result: Arc<dyn KernelObject> =
                    if child.object_type() == ObjectType::Directory { wrap_child_directory(child, self.arc_clone()) } else { child };

                register_result(result, child_effective_rights)
            }
        }
    }

    async fn create_dir(&self, name_ptr: usize, name_len: usize) -> Result<usize, InvocationError> {
        let filename = crate::core::object::models::directory::Filename::new(name_ptr as *const u8, name_len)?;
        let owner = get_current_process().ok_or(InvocationError::InvalidHandle)?.credentials.user();
        let directory = self.directory();
        let child = directory.as_directory().ok_or(InvocationError::InvalidArgument)?.create_child_dir(&filename.name, owner).await?;

        if child.object_type() != ObjectType::Directory {
            return Err(InvocationError::InvalidArgument);
        }

        let child_rights = allowed_rights(&child)?;
        let location = DirLocation::child(child, self.arc_clone());
        register_result(location, child_rights)
    }
}

#[async_trait]
impl KernelObject for DirLocation {
    fn type_name(&self) -> &'static str { "Directory" }

    fn object_type(&self) -> ObjectType { ObjectType::Directory }

    fn as_any(&self) -> &dyn core::any::Any { self }

    async fn invoke(&self, invocation: Invocation, rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Resolve { start, path_ptr, path_len, rights: requested_rights }) => {
                let path = copy_path(path_ptr, path_len)?;
                self.resolve(start, &path, requested_rights, rights).await
            }
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => self.lookup(name, name_len, rights).await,
            Invocation::Directory(DirectoryOp::CreateDir { name, name_len }) => self.create_dir(name, name_len).await,
            other => self.dir.invoke(other, rights).await,
        }
    }

    fn permissions(&self) -> Option<FilePermissions> { self.dir.permissions() }
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

fn ancestry_from(root: &Arc<DirLocation>, start: &Arc<DirLocation>) -> Result<Vec<Arc<DirLocation>>, InvocationError> {
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
    Ok(reversed)
}

fn wrap_child_directory(child: Arc<dyn KernelObject>, parent: Arc<DirLocation>) -> Arc<DirLocation> { DirLocation::child(child, parent) }

fn register_result(object: Arc<dyn KernelObject>, rights: AccessRights) -> Result<usize, InvocationError> {
    let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
    Ok(proc.proc_handles.write().insert(object, rights).0)
}

pub async fn resolve_kernel_object(
    _authority: &KernelNamespaceAuthority, root: Arc<DirLocation>, path: &str,
) -> Result<Arc<dyn KernelObject>, InvocationError> {
    if path.len() > PATH_MAX {
        return Err(InvocationError::NameTooLong);
    }

    let mut ancestry = vec![root.clone()];
    let mut resolved = KernelResolved::Directory(root);

    for component in Components::new(path) {
        match component {
            Component::Root => {
                ancestry.truncate(1);
                resolved = KernelResolved::Directory(ancestry[0].clone());
            }

            Component::Current => {
                if !matches!(resolved, KernelResolved::Directory(_)) {
                    return Err(InvocationError::InvalidArgument);
                }
            }

            Component::Parent => {
                if !matches!(resolved, KernelResolved::Directory(_)) {
                    return Err(InvocationError::InvalidArgument);
                }

                if ancestry.len() > 1 {
                    ancestry.pop();
                }

                resolved = KernelResolved::Directory(ancestry.last().ok_or(InvocationError::InvalidArgument)?.clone());
            }

            Component::Normal(name) => {
                if !matches!(resolved, KernelResolved::Directory(_)) {
                    return Err(InvocationError::InvalidArgument);
                }

                let current = ancestry.last().ok_or(InvocationError::InvalidArgument)?;

                let directory = current.directory();
                let child = directory.as_directory().ok_or(InvocationError::InvalidArgument)?.lookup_child(name).await?;

                if child.object_type() == ObjectType::Directory {
                    let location = DirLocation::child(child, current.clone());

                    ancestry.push(location.clone());
                    resolved = KernelResolved::Directory(location);
                } else {
                    resolved = KernelResolved::Object(child);
                }
            }
        }
    }

    Ok(match resolved {
        KernelResolved::Directory(location) => location.directory(),
        KernelResolved::Object(object) => object,
    })
}
