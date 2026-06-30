use alloc::sync::Arc;

use vespertine_abi::{
    AccessRights,
    UserID,
};

use crate::object::invoke::InvocationError;
use crate::object::obj::KernelObject;
use crate::process::current_process;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilePermissions {
    pub owner: UserID,
    pub owner_rights: AccessRights,
    pub other_rights: AccessRights,
}

impl FilePermissions {
    pub fn allowed_for(self, user: UserID) -> AccessRights { if user == self.owner { self.owner_rights } else { self.other_rights } }
}

pub fn allowed_rights(object: &Arc<dyn KernelObject>) -> Result<AccessRights, InvocationError> {
    let proc = current_process().ok_or(InvocationError::InvalidHandle)?;

    let ret = match object.permissions() {
        Some(perms) => perms.allowed_for(proc.credentials.user()),
        None => AccessRights::all(), // virtual objects remain capability-controlled
    };

    Ok(ret)
}

pub fn rights_for_permissions(permissions: FilePermissions) -> Result<AccessRights, InvocationError> {
    let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
    Ok(permissions.allowed_for(proc.credentials.user()))
}
