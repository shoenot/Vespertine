use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;

pub use vespertine_abi::{
    AccessRights,
    HandleID,
};

use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::{
    HandleEntry,
    KernelObject,
};

#[derive(Debug)]
pub struct HandleTable {
    pub entries: BTreeMap<HandleID, HandleEntry>,
    next_id: usize,
}

impl HandleTable {
    pub const fn new() -> Self { Self { entries: BTreeMap::new(), next_id: 100 } }

    pub fn insert(&mut self, obj: Arc<dyn KernelObject>, rights: AccessRights) -> HandleID {
        let id = HandleID(self.next_id);
        self.next_id += 1;
        self.entries.insert(id, HandleEntry { rights, object: obj });
        id
    }

    pub fn insert_at(&mut self, id: HandleID, obj: Arc<dyn KernelObject>, rights: AccessRights) {
        self.entries.insert(id, HandleEntry { rights, object: obj });
        if id.0 >= self.next_id {
            self.next_id = id.0 + 1;
        }
    }

    pub fn get(&self, id: &HandleID) -> Option<&HandleEntry> { self.entries.get(id) }

    pub fn close(&mut self, id: HandleID) -> Result<(), InvocationError> {
        match self.entries.remove(&id) {
            Some(_) => Ok(()),
            None => Err(InvocationError::InvalidHandle),
        }
    }

    pub fn resolve_entry(&self, id: HandleID, required: AccessRights) -> Result<&HandleEntry, InvocationError> {
        let entry = self.entries.get(&id).ok_or(InvocationError::InvalidHandle)?;
        if !entry.rights.contains(required) {
            return Err(InvocationError::AccessDenied);
        }
        Ok(entry)
    }

    pub fn resolve(&self, id: HandleID, required: AccessRights) -> Result<Arc<dyn KernelObject>, InvocationError> {
        let entry = self.resolve_entry(id, required)?;
        Ok(entry.object.clone())
    }

    pub fn duplicate(&mut self, id: HandleID, requested_rights: AccessRights) -> Result<HandleID, InvocationError> {
        let entry = self.entries.get(&id).ok_or(InvocationError::InvalidHandle)?;
        if !entry.rights.contains(requested_rights) {
            return Err(InvocationError::AccessDenied);
        }
        let obj_clone = entry.object.clone();
        Ok(self.insert(obj_clone, requested_rights))
    }

    pub fn clear(&mut self) { self.entries.clear(); }
}
