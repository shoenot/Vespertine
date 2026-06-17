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
use vespertine_abi::op::{DirectoryOp, FileOp};
use vespertine_abi::protocol::{
    AbiDirEntry,
    DirEntryType,
    PacketFlags,
    PacketHeader,
    VESPER_MAGIC,
};
use vespertine_abi::{
    AccessRights, FileStat, HandleID, Invocation, ObjectType
};

use crate::arch::x86_64::task::syscall::{safe_copy_from, safe_copy_to};
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::{
    KernelDirectory,
    KernelObject,
};
use crate::core::sync::RwLock;
use crate::core::thread::get_current_process;
use crate::memory::NORMAL_PAGE_SIZE;

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

pub fn validate_child_name(name: &str) -> Result<(), InvocationError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(InvocationError::InvalidArgument);
    }
    if name.len() > FILENAME_LEN_MAX {
        return Err(InvocationError::NameTooLong);
    }
    Ok(())
}

fn register_lookup_result(object: Arc<dyn KernelObject>) -> Result<usize, InvocationError> {
    let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
    Ok(proc.proc_handles.write().insert(object, AccessRights::all()).0)
}

#[async_trait]
impl KernelObject for Directory {
    async fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Link { .. }) => Err(InvocationError::UnsupportedOperation),
            Invocation::Directory(DirectoryOp::Unlink { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                KernelDirectory::unlink_child(self, &filename.name).await?;
                Ok(0)
            }
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                let object = KernelDirectory::lookup_child(self, &filename.name).await?;
                register_lookup_result(object)
            }
            Invocation::Directory(DirectoryOp::List { offset, sink }) => self.list_contents(offset, sink).await,
            Invocation::File(FileOp::Stat { stat_ptr }) => {
                let stat = FileStat {
                    object_type: ObjectType::Directory as u32,
                    mode: 0x4000 | 0o755,
                    user: 0,
                    _group: 0,
                    inode: self as *const _ as u64,
                    device: 0,
                    size: 0 as u64,
                    block_size: NORMAL_PAGE_SIZE as u32,
                    blocks: 0,
                    nlink: 1,
                    atime_sec: 0,
                    atime_nsec: 0,
                    mtime_sec: 0,
                    mtime_nsec: 0,
                    ctime_sec: 0,
                    ctime_nsec: 0,
                };

                if !safe_copy_to(stat_ptr as *mut u8, &stat as *const _ as *const u8, size_of::<FileStat>()) {
                    return Err(InvocationError::InvalidPointer);
                }
                Ok(0)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn as_directory(&self) -> Option<&dyn KernelDirectory> { Some(self) }

    fn type_name(&self) -> &'static str { "Directory" }

    fn object_type(&self) -> ObjectType { ObjectType::Directory }
}

#[async_trait]
impl KernelDirectory for Directory {
    async fn lookup_child(&self, name: &str) -> Result<Arc<dyn KernelObject>, InvocationError> {
        validate_child_name(name)?;
        self.tree.read().get(name).cloned().ok_or(InvocationError::PathNotFound)
    }

    async fn link_child(&self, name: &str, object: Arc<dyn KernelObject>) -> Result<(), InvocationError> {
        validate_child_name(name)?;
        let filename = Filename { name: Box::from(name) };
        if self.tree.write().insert(filename, object).is_some() {
            return Err(InvocationError::InvalidArgument);
        }
        Ok(())
    }

    async fn unlink_child(&self, name: &str) -> Result<(), InvocationError> {
        validate_child_name(name)?;
        self.tree.write().remove(name).ok_or(InvocationError::PathNotFound)?;
        Ok(())
    }
}

impl Directory {
    pub const fn new() -> Self { Self { tree: RwLock::new(BTreeMap::new()) } }

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
