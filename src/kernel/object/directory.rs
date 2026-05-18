use alloc::{collections::btree_map::BTreeMap, string::String};

use crate::kernel::{object::{handle::HandleID, invoke::{DirectoryMessage, Invocation, InvocationError}, obj::KernelObject}, sync::RwLock};


#[derive(Debug)]
pub struct Directory {
    tree: RwLock<BTreeMap<String, HandleID>>,
}

impl KernelObject for Directory {
    fn invoke(&self, invocation: Invocation) -> Result<(), InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryMessage::Link { name, handle_id }) => { self.link(name, handle_id) },
            Invocation::Directory(DirectoryMessage::Unlink { name }) => { self.unlink(&name) },
            Invocation::Directory(DirectoryMessage::Lookup { name }) => { self.lookup(&name) },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl Directory {
    pub const fn new() -> Self {
        Self { tree: RwLock::new(BTreeMap::new()) }
    }

    fn link(&self, name: String, handle_id: HandleID) -> Result<(), InvocationError> {
        self.tree.write().insert(name, handle_id);
        Ok(())
    }

    fn unlink(&self, name: &String) -> Result<(), InvocationError> {
        self.tree.write().remove_entry(name);
        Ok(())
    }

    fn lookup(&self, name: &String) -> Result<HandleID, InvocationError> {
        self.tree.read().get(name).copied().ok_or(InvocationError::InvalidHandle)
    }
}
