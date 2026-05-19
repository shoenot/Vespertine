use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::{
    slice,
    str,
};
use core::borrow::Borrow;
use core::str::Utf8Error;

use crate::kernel::object::handle::HandleID;
use crate::kernel::object::invoke::{
    Invocation,
    InvocationError,
};
use crate::kernel::object::op::DirectoryOp;
use crate::kernel::object::obj::KernelObject;
use crate::kernel::sync::RwLock;

#[derive(Debug)]
pub struct Directory {
    tree: RwLock<BTreeMap<Filename, HandleID>>,
}

#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct Filename {
    name: Box<str>,
}

impl Borrow<str> for Filename {
    fn borrow(&self) -> &str { &self.name }
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
        self.tree.write().insert(filename, handle_id);
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
        match self.tree.read().get(name_str).copied() {
            Some(h) => Ok(h.0),
            None => Err(InvocationError::InvalidHandle),
        }
    }
}
