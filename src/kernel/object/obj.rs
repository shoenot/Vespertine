use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use core::fmt::{
    Debug,
    Display,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::kernel::object::handle::{
    AccessRights,
    HandleID,
};
use crate::kernel::object::invoke::{
    Invocation,
    InvocationError,
};

pub trait KernelObject: Send + Sync + Debug {
    fn invoke(&self, invocation: Invocation) -> Result<(), InvocationError>;

    fn type_name(&self) -> &'static str { "Unknown" }
}

#[derive(Debug)]
pub struct HandleEntry {
    pub rights: AccessRights,
    pub object: Arc<dyn KernelObject>,
}

#[derive(Debug)]
pub struct KernelHandleTable {
    next_id: AtomicUsize,
    entries: BTreeMap<HandleID, HandleEntry>,
}

impl KernelHandleTable {
    pub const fn new() -> Self { Self { next_id: AtomicUsize::new(1), entries: BTreeMap::new() } }

    pub fn insert(&mut self, object: Arc<dyn KernelObject>, rights: AccessRights) -> HandleID {
        let id = HandleID(self.next_id.fetch_add(1, Ordering::Relaxed));
        let entry = HandleEntry { rights, object };
        self.entries.insert(id, entry);
        id
    }

    pub fn resolve(&self, id: HandleID, required_rights: AccessRights) -> Result<Arc<dyn KernelObject>, InvocationError> {
        let entry = self.entries.get(&id).ok_or(InvocationError::InvalidHandle)?;

        if !entry.rights.contains(required_rights) {
            return Err(InvocationError::AccessDenied);
        }

        // return cloned arc to bump ref count safely
        Ok(entry.object.clone())
    }

    pub fn close(&mut self, id: HandleID) -> Result<(), InvocationError> {
        self.entries.remove(&id).ok_or(InvocationError::InvalidHandle)?;
        Ok(())
    }
}

impl Display for KernelHandleTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for item in &self.entries {
            write!(f, "{:#?}", item);
        }
        Ok(())
    }
}
