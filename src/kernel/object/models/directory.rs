use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::{
    slice,
    str,
};
use core::borrow::Borrow;
use core::str::Utf8Error;
use crate::arch::get_core_data;
use crate::kernel::object::handle::{AccessRights, HandleID};
use crate::kernel::object::invoke::{
    Invocation,
    InvocationError,
};
use crate::kernel::object::op::DirectoryOp;
use crate::kernel::object::obj::{HandleEntry, KernelObject};
use crate::kernel::process::pcb::get_new_pid;
use crate::kernel::sync::RwLock;

#[derive(Debug)]
pub struct Directory {
    tree: RwLock<BTreeMap<Filename, Arc<dyn KernelObject>>>,
}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Filename {
    name: Box<str>,
}

impl Borrow<str> for Filename {
    fn borrow(&self) -> &str { &self.name }
}

impl PartialEq<str> for Filename {
    fn eq(&self, other: &str) -> bool {
        &*self.name == other
    }
}

impl PartialOrd<str> for Filename {
    fn partial_cmp(&self, other: &str) -> Option<core::cmp::Ordering> {
        self.name.as_ref().partial_cmp(other)
    }
}

impl Filename {
    pub fn new(ptr: *const u8, len: usize) -> Result<Self, Utf8Error> {
        unsafe {
            let name_bytes = slice::from_raw_parts(ptr, len);
            let name_str = match str::from_utf8(name_bytes) {
                Ok(s) => s,
                Err(e) => return Err(e),
            };
            Ok(Self { name: Box::from(name_str) })
        }
    }
}

impl KernelObject for Directory {
    fn invoke(&self, invocation: Invocation) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Link { name, name_len, handle_id }) => self.link(name, name_len, handle_id),
            Invocation::Directory(DirectoryOp::Unlink { name, name_len }) => self.unlink(name, name_len),
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => self.lookup(name, name_len),
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl Directory {
    pub const fn new() -> Self { Self { tree: RwLock::new(BTreeMap::new()) } }

    fn link(&self, name: *const u8, name_len: usize, handle_id: HandleID) -> Result<usize, InvocationError> {
        let filename = Filename::new(name, name_len)?;
        let current_thread = get_core_data().scheduler.get_current_thread();
        let proc = unsafe { &(*current_thread).process };

        let table = proc.proc_handles.read();
        let entry = table.get(&handle_id).ok_or(InvocationError::InvalidHandle)?;
        let obj_arc = entry.object.clone();

        self.tree.write().insert(filename, obj_arc);
        Ok(0)
    }

    fn unlink(&self, name: *const u8, name_len: usize) -> Result<usize, InvocationError> {
        let name_str = unsafe {
            let name_bytes = slice::from_raw_parts(name, name_len);
            str::from_utf8(name_bytes)?
        };
        self.tree.write().remove_entry(name_str);
        Ok(0)
    }

    fn lookup(&self, name: *const u8, name_len: usize) -> Result<usize, InvocationError> {
        let name_str = unsafe {
            let name_bytes = slice::from_raw_parts(name, name_len);
            str::from_utf8(name_bytes)?
        };

        let obj_arc = match self.tree.read().get(name_str) {
            Some(obj) => obj.clone(),
            None => return Err(InvocationError::InvalidArgument),
        };

        let current_thread = get_core_data().scheduler.get_current_thread();
        let proc = unsafe { &(*current_thread).process };

        let mut table = proc.proc_handles.write();
        let new_id = get_new_pid();
        table.insert(HandleID(new_id), HandleEntry { rights: AccessRights::MUTATE | AccessRights::READ | AccessRights::WRITE, object: obj_arc });

        Ok(new_id)
    }
}
