use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::{
    String,
    ToString,
};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp;
use core::ptr::copy_nonoverlapping;

use async_trait::async_trait;
use vespertine_abi::protocol::{
    AbiDirEntry,
    DirEntryType,
    PacketFlags,
    PacketHeader,
    VESPER_MAGIC,
};
use vespertine_abi::{
    AccessRights,
    DirectoryOp,
    FileOp,
    FileStat,
    Invocation,
    ObjectType,
    UserID,
};

use crate::arch::x86_64::task::syscall::safe_copy_to;
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::directory::{
    Filename,
    validate_child_name,
};
use crate::core::object::obj::{
    KernelDirectory,
    KernelObject,
};
use crate::core::security::permissions::{
    FilePermissions,
    allowed_rights,
};
use crate::core::sync::RwLock;
use crate::core::thread::get_current_process;
use crate::memory::NORMAL_PAGE_SIZE;

#[derive(Debug)]
pub struct MountDirectory {
    pub underlying: RwLock<Arc<dyn KernelObject>>,
    pub overlays: RwLock<BTreeMap<Filename, Arc<dyn KernelObject>>>,
}

impl MountDirectory {
    pub fn new(underlying: Arc<dyn KernelObject>) -> Self {
        Self { underlying: RwLock::new(underlying), overlays: RwLock::new(BTreeMap::new()) }
    }

    pub fn set_underlying(&self, new_underlying: Arc<dyn KernelObject>) { *self.underlying.write() = new_underlying; }
}

#[async_trait]
impl KernelObject for MountDirectory {
    fn type_name(&self) -> &'static str { "Directory" }

    fn as_any(&self) -> &dyn core::any::Any { self }

    fn object_type(&self) -> ObjectType { ObjectType::Directory }

    fn as_directory(&self) -> Option<&dyn KernelDirectory> { Some(self) }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                let object = KernelDirectory::lookup_child(self, &filename.name).await?;
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.proc_handles.write().insert(object, AccessRights::all());
                Ok(handle.0)
            },
            Invocation::Directory(DirectoryOp::Link { name, name_len, handle_id }) => {
                calling_rights.err_if_no(AccessRights::CREATE)?;

                let filename = Filename::new(name as *const u8, name_len)?;
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let object = {
                    let handles = proc.proc_handles.read();
                    handles.resolve(handle_id, AccessRights::READ)?
                };

                KernelDirectory::link_child(self, &filename.name, object).await?;
                Ok(0)
            },
            Invocation::Directory(DirectoryOp::Unlink { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                KernelDirectory::unlink_child(self, &filename.name).await?;
                Ok(0)
            },
            Invocation::File(FileOp::Stat { stat_ptr }) => {
                let underlying = self.underlying.read().clone();
                match underlying.invoke(Invocation::File(FileOp::Stat { stat_ptr }), calling_rights).await {
                    Ok(v) => Ok(v),
                    Err(InvocationError::UnsupportedOperation) => {
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
                    }
                    Err(e) => Err(e),
                }
            },
            Invocation::Directory(DirectoryOp::List { offset, sink }) => {
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let sink_obj = proc.proc_handles.read().resolve(sink, AccessRights::WRITE)?;

                let overlays_entries: Vec<(String, &'static str)> = {
                    let overlays = self.overlays.read();
                    overlays.iter().map(|(name, obj)| (name.name.to_string(), obj.type_name())).collect()
                };

                let underlying = self.underlying.read().clone();

                // stream virtual overlays first
                let mut iter = overlays_entries.iter().peekable();
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

                    let mut flags = PacketFlags::IS_STREAM;
                    // set has next because the underlying disk will stream after the overlays
                    if iter.peek().is_some() {
                        flags = flags.insert(PacketFlags::HAS_NEXT);
                    }

                    let header = PacketHeader {
                        magic: VESPER_MAGIC,
                        version: 1,
                        packet_flags: flags,
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
                        return Ok(0); // sink disconnected
                    }
                }

                // stream underlying disk dirs
                let _ = underlying.invoke(Invocation::Directory(DirectoryOp::List { offset, sink }), calling_rights).await;

                Ok(0)
            },
            Invocation::Directory(DirectoryOp::CreateFile { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                let owner = get_current_process().ok_or(InvocationError::InvalidHandle)?.credentials.user();
                let object = KernelDirectory::create_child_file(self, &filename.name, owner).await?;
                let rights = allowed_rights(&object)?;
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                Ok(proc.proc_handles.write().insert(object, rights).0)
            },
            Invocation::Directory(DirectoryOp::CreateDir { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                let owner = get_current_process().ok_or(InvocationError::InvalidHandle)?.credentials.user();
                let object = KernelDirectory::create_child_dir(self, &filename.name, owner).await?;
                let rights = allowed_rights(&object)?;
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                Ok(proc.proc_handles.write().insert(object, rights).0)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn permissions(&self) -> Option<FilePermissions> { self.underlying.read().permissions() }
}

#[async_trait]
impl KernelDirectory for MountDirectory {
    async fn lookup_child(&self, name: &str) -> Result<Arc<dyn KernelObject>, InvocationError> {
        if let Some(object) = self.overlays.read().get(name).cloned() {
            return Ok(object);
        }

        let underlying = self.underlying.read().clone();
        underlying.as_directory().ok_or(InvocationError::UnsupportedOperation)?.lookup_child(name).await
    }

    async fn create_child_file(&self, name: &str, owner: UserID) -> Result<Arc<dyn KernelObject>, InvocationError> {
        let underlying = self.underlying.read().clone();
        underlying.as_directory().ok_or(InvocationError::UnsupportedOperation)?.create_child_file(name, owner).await
    }

    async fn create_child_dir(&self, name: &str, owner: UserID) -> Result<Arc<dyn KernelObject>, InvocationError> {
        let underlying = self.underlying.read().clone();
        underlying.as_directory().ok_or(InvocationError::UnsupportedOperation)?.create_child_dir(name, owner).await
    }

    async fn link_child(&self, name: &str, object: Arc<dyn KernelObject>) -> Result<(), InvocationError> {
        validate_child_name(name)?;
        let filename = Filename { name: Box::from(name) };
        let mut overlays = self.overlays.write();
        if overlays.contains_key(&filename) {
            return Err(InvocationError::InvalidArgument);
        }
        overlays.insert(filename, object);
        Ok(())
    }

    async fn unlink_child(&self, name: &str) -> Result<(), InvocationError> {
        validate_child_name(name)?;
        if self.overlays.write().remove(name).is_some() {
            return Ok(());
        }
        let underlying = self.underlying.read().clone();
        underlying.as_directory().ok_or(InvocationError::UnsupportedOperation)?.unlink_child(name).await
    }
}
