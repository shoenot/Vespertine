use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::{
    String,
    ToString,
};
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::{
    slice,
    str,
};
use core::borrow::Borrow;
use core::cmp;
use core::ptr::copy_nonoverlapping;

use async_trait::async_trait;
use vespertine_abi::op::DirectoryOp;
use vespertine_abi::protocol::{
    AbiDirEntry,
    DirEntryType,
    PacketFlags,
    PacketHeader,
    VESPER_MAGIC,
};
use vespertine_abi::{
    AccessRights,
    FileOp,
    HandleID,
    Invocation,
};

use crate::arch::get_core_data;
use crate::arch::x86_64::task::syscall::safe_copy_from;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::core::sync::RwLock;
use crate::core::thread::get_current_process;

pub const FILENAME_LEN_MAX: usize = 254;

#[derive(Debug)]
pub struct Directory {
    tree: RwLock<BTreeMap<Filename, Arc<dyn KernelObject>>>,
}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Filename {
    pub name: Box<str>,
}

impl Borrow<str> for Filename {
    fn borrow(&self) -> &str { &self.name }
}

impl PartialEq<str> for Filename {
    fn eq(&self, other: &str) -> bool { &*self.name == other }
}

impl PartialOrd<str> for Filename {
    fn partial_cmp(&self, other: &str) -> Option<core::cmp::Ordering> { self.name.as_ref().partial_cmp(other) }
}

impl Filename {
    pub fn new(ptr: *const u8, len: usize) -> Result<Self, InvocationError> {
        if len > FILENAME_LEN_MAX {
            return Err(InvocationError::NameTooLong);
        };
        let mut filename = [0u8; 255];
        let filename_ptr = filename.as_mut_ptr();

        let name_str = unsafe {
            if !safe_copy_from(filename_ptr, ptr, len) {
                return Err(InvocationError::InvalidPointer);
            }
            let name_bytes = slice::from_raw_parts(filename_ptr, len);
            str::from_utf8(name_bytes)?
        };
        Ok(Self { name: Box::from(name_str) })
    }
}

#[async_trait]
impl KernelObject for Directory {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Link { name, name_len, handle_id }) => self.link(name as *const u8, name_len, handle_id),
            Invocation::Directory(DirectoryOp::Unlink { name, name_len }) => self.unlink(name as *const u8, name_len),
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => self.lookup(name as *const u8, name_len, calling_rights),
            Invocation::Directory(DirectoryOp::List { offset, sink }) => self.list_contents(offset, sink).await,
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str { "Directory" }
}

impl Directory {
    pub const fn new() -> Self { Self { tree: RwLock::new(BTreeMap::new()) } }

    fn link(&self, name: *const u8, name_len: usize, handle_id: HandleID) -> Result<usize, InvocationError> {
        let filename = Filename::new(name, name_len)?;
        let current_thread = get_core_data().scheduler.get_current_thread();
        let proc = unsafe { &(*current_thread).process };

        let obj_arc = {
            let table = proc.proc_handles.read();
            let entry = table.get(&handle_id).ok_or(InvocationError::InvalidHandle)?;
            entry.object.clone()
        };

        self.tree.write().insert(filename, obj_arc);
        Ok(0)
    }

    fn unlink(&self, name: *const u8, name_len: usize) -> Result<usize, InvocationError> {
        let filename = Filename::new(name, name_len)?.name;
        self.tree.write().remove_entry(&*filename);
        Ok(0)
    }

    fn lookup(&self, name: *const u8, name_len: usize, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        if name_len > FILENAME_LEN_MAX {
            return Err(InvocationError::NameTooLong);
        };
        let mut filename = [0u8; 255];
        let _filename_ptr = filename.as_mut_ptr();
        let name_str = Filename::new(name, name_len)?.name;

        let obj_arc = {
            let tree = self.tree.read();
            match tree.get(&*name_str) {
                Some(obj) => obj.clone(),
                None => return Err(InvocationError::PathNotFound),
            }
        };

        let rights = AccessRights(
            calling_rights.0 &
                (AccessRights::MUTATE | AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE).0,
        );

        let handle_id = get_current_process().ok_or(InvocationError::InvalidHandle)?.proc_handles.write().insert(obj_arc, rights);
        Ok(handle_id.0)
    }

    async fn list_contents(&self, _offset: usize, sink: HandleID) -> Result<usize, InvocationError> {
        let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;

        let sink_obj = proc.proc_handles.read().resolve(sink, AccessRights::WRITE)?;

        let entries: Vec<(String, &'static str)> = {
            let tree = self.tree.read();
            tree.iter().map(|(name, obj)| (name.name.to_string(), obj.type_name())).collect()
        }; // drop read lock

        crate::core::asynchronous::Executor::new().spawn(async move {
            let mut iter = entries.iter().peekable();
            while let Some((name_str, type_name)) = iter.next() {
                let mut entry = AbiDirEntry {
                    entry_type: match *type_name {
                        "Directory" => DirEntryType::Directory as u8,
                        "File" => DirEntryType::File as u8,
                        _ => DirEntryType::Object as u8,
                    },
                    name_len: cmp::min(name_str.len(), 254) as u8,
                    name: [0u8; 254],
                };
                let len = entry.name_len as usize;
                entry.name[..len].copy_from_slice(&name_str.as_bytes()[..len]);

                // Dynamically set HAS_NEXT if there are more entries in the vector
                let mut flags = PacketFlags::IS_STREAM;
                if iter.peek().is_some() {
                    flags = flags.insert(PacketFlags::HAS_NEXT);
                }

                let header = PacketHeader {
                    magic: VESPER_MAGIC,
                    version: 1,
                    packet_flags: flags, // Use the correct flags
                    packet_type: 1,
                    payload_len: size_of::<AbiDirEntry>() as u32,
                    reserved: 0,
                };

                let mut buffer = [0u8; size_of::<PacketHeader>() + size_of::<AbiDirEntry>()];
                let header_size = size_of::<PacketHeader>();
                let entry_size = size_of::<AbiDirEntry>();
                unsafe {
                    let header_ptr = &header as *const _ as *const u8;
                    let entry_ptr = &entry as *const _ as *const u8;
                    copy_nonoverlapping(header_ptr, buffer.as_mut_ptr(), header_size);
                    copy_nonoverlapping(entry_ptr, buffer.as_mut_ptr().add(header_size), entry_size);
                }

                let op = FileOp::Write { offset: 0, buffer_ptr: buffer.as_mut_ptr() as usize, len: buffer.len() };
                if sink_obj.invoke(Invocation::File(op), AccessRights::WRITE).await.is_err() {
                    break;
                }
            }
        });

        Ok(0)
    }
}
