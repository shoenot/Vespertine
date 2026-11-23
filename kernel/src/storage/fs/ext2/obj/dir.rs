use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::{ptr, slice, str};
use core::ptr::copy_nonoverlapping;

use async_trait::async_trait;
use hal::usercopy::safe_copy_to;
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

use super::file::Ext2File;
use crate::klogln;
use crate::object::invoke::InvocationError;
use crate::object::fs::directory::{
    FILENAME_LEN_MAX,
    Filename,
    validate_child_name,
};
use crate::object::obj::{
    KernelDirectory,
    KernelObject,
};
use crate::object::vfs::{FileDescription, write_all_to_object};
use crate::security::permissions::{
    FilePermissions,
    allowed_rights,
};
use crate::sync::RwLock;
use crate::process::current_process;
use crate::time::get_realtime;
use crate::memory::PhysBuffer;
use crate::storage::fs::ext2::{Ext2Error, Ext2FileSystem};
use crate::storage::fs::ext2::permissions::directory_permissions;
use crate::storage::fs::ext2::structs::{
    DiskDirHeader,
    DiskInode,
};

#[derive(Debug)]
pub struct Ext2Directory {
    pub fs: Arc<Ext2FileSystem>,
    pub inode_num: u32,
    pub inode_data: RwLock<DiskInode>,
}

#[async_trait]
impl KernelObject for Ext2Directory {
    fn type_name(&self) -> &'static str { "Directory" }

    fn object_type(&self) -> ObjectType { ObjectType::Directory }

    fn as_directory(&self) -> Option<&dyn KernelDirectory> { Some(self) }

    async fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                let object = KernelDirectory::lookup_child(self, &filename.name).await?;

                let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.handles.write().insert(object, AccessRights::all());

                Ok(handle.0)
            }
            Invocation::File(FileOp::Stat { stat_ptr }) => {
                let inode = self.inode_data.read();

                let stat = FileStat {
                    object_type: ObjectType::Directory as u32,
                    mode: inode.mode as u32,
                    user: inode.uid as u32,
                    _group: 0,
                    inode: self.inode_num as u64,
                    device: 1,
                    size: inode.size as u64,
                    block_size: self.fs.block_size as u32,
                    blocks: inode.blocks as u64,
                    nlink: inode.links_count as u32,
                    atime_sec: inode.atime as i64,
                    atime_nsec: 0,
                    mtime_sec: inode.mtime as i64,
                    mtime_nsec: 0,
                    ctime_sec: inode.ctime as i64,
                    ctime_nsec: 0,
                };

                if !safe_copy_to(stat_ptr as *mut u8, &stat as *const _ as *const u8, size_of::<FileStat>()) {
                    return Err(InvocationError::InvalidPointer);
                }

                Ok(0)
            }
            Invocation::Directory(DirectoryOp::List { offset: _, sink }) => {
                let entries = self.list_entries().await?;

                // resolve sink socket
                let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
                let sink_obj = proc.handles.read().resolve(sink, AccessRights::WRITE)?;

                crate::executor::Executor::new().spawn(async move {
                    let mut iter = entries.iter().peekable();
                    while let Some((name_str, file_type)) = iter.next() {
                        let mut entry = AbiDirEntry {
                            entry_type: match *file_type {
                                2 => DirEntryType::Directory as u8,
                                1 => DirEntryType::File as u8,
                                _ => DirEntryType::Object as u8,
                            },
                            name_len: core::cmp::min(name_str.len(), 254) as u8,
                            name: [0u8; 254],
                        };
                        let len = entry.name_len as usize;
                        entry.name[..len].copy_from_slice(&name_str.as_bytes()[..len]);

                        let mut flags = PacketFlags::IS_STREAM;
                        if iter.peek().is_some() {
                            flags = flags.insert(PacketFlags::HAS_NEXT);
                        }

                        let header = PacketHeader {
                            magic: VESPER_MAGIC,
                            version: 1,
                            packet_flags: flags,
                            packet_type: 1,
                            payload_len: core::mem::size_of::<AbiDirEntry>() as u32,
                            reserved: 0,
                        };

                        let mut buffer = [0u8; core::mem::size_of::<PacketHeader>() + core::mem::size_of::<AbiDirEntry>()];
                        let header_size = core::mem::size_of::<PacketHeader>();
                        let entry_size = core::mem::size_of::<AbiDirEntry>();
                        unsafe {
                            let header_ptr = &header as *const _ as *const u8;
                            let entry_ptr = &entry as *const _ as *const u8;
                            copy_nonoverlapping(header_ptr, buffer.as_mut_ptr(), header_size);
                            copy_nonoverlapping(entry_ptr, buffer.as_mut_ptr().add(header_size), entry_size);
                        }

                        if write_all_to_object(sink_obj.as_ref(), AccessRights::WRITE, &buffer).await.is_err() {
                            break;
                        }
                    }
                });

                Ok(0)
            }

            Invocation::Directory(DirectoryOp::Link { .. }) => Err(InvocationError::UnsupportedOperation),

            Invocation::Directory(DirectoryOp::Unlink { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                KernelDirectory::unlink_child(self, &filename.name).await?;
                Ok(0)
            }
            Invocation::Directory(DirectoryOp::CreateFile { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                let owner = current_process().ok_or(InvocationError::InvalidHandle)?.credentials.user();
                let object = KernelDirectory::create_child_file(self, &filename.name, owner).await?;
                register_created_object(object)
            }

            Invocation::Directory(DirectoryOp::CreateDir { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;
                let owner = current_process().ok_or(InvocationError::InvalidHandle)?.credentials.user();
                let object = KernelDirectory::create_child_dir(self, &filename.name, owner).await?;
                register_created_object(object)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn permissions(&self) -> Option<FilePermissions> {
        let inode = self.inode_data.read();
        Some(directory_permissions(inode.uid, inode.mode))
    }
}

#[async_trait]
impl KernelDirectory for Ext2Directory {
    async fn lookup_child(&self, name: &str) -> Result<Arc<dyn KernelObject>, InvocationError> {
        if name.len() > FILENAME_LEN_MAX {
            return Err(InvocationError::NameTooLong);
        }

        let _namespace = self.fs.namespace_lock.lock().await;
        let parent = *self.inode_data.read();
        let child_inode_id = self.fs.lookup_in_dir(&parent, name).await?
            .ok_or(InvocationError::PathNotFound)?;

        if let Some(file) = Ext2File::active(&self.fs, child_inode_id) {
            return Ok(Arc::new(FileDescription::new(file)));
        }

        let child_inode_data = self.fs.read_inode(child_inode_id).await?;

        let is_directory = (child_inode_data.mode & 0xF000) == 0x4000;

        let target_object: Arc<dyn KernelObject> = if is_directory {
            let mut dirs = self.fs.active_dirs.lock();
            let mut cached = None;
            if let Some(weak_dir) = dirs.get(&child_inode_id) {
                cached = weak_dir.upgrade();
            }
            if let Some(arc_dir) = cached {
                arc_dir
            } else {
                let new_dir = Arc::new(Ext2Directory {
                    fs: Arc::clone(&self.fs),
                    inode_num: child_inode_id,
                    inode_data: RwLock::new(child_inode_data),
                });
                dirs.insert(child_inode_id, Arc::downgrade(&new_dir));
                new_dir
            }
        } else {
            if child_inode_data.links_count == 0 {
                return Err(Ext2Error::InvalidMetadata.into());
            }
            let file = Ext2File::get_or_open(&self.fs, child_inode_id, child_inode_data);
            Arc::new(FileDescription::new(file))
        };

        Ok(target_object)
    }

    async fn unlink_child(&self, name: &str) -> Result<(), InvocationError> {
        validate_child_name(name)?;

        let _namespace = self.fs.namespace_lock.lock().await;
        let parent = *self.inode_data.read();
        let inode_num = self.fs.lookup_in_dir(&parent, name).await?
            .ok_or(InvocationError::PathNotFound)?;

        let file = if let Some(file) = Ext2File::active(&self.fs, inode_num) {
            file
        } else {
            let inode = self.fs.read_inode(inode_num).await?;
            if inode.mode & 0xf000 != 0x8000 {
                return Err(InvocationError::UnsupportedOperation);
            }
            if inode.links_count == 0 {
                return Err(Ext2Error::InvalidMetadata.into());
            }
            Ext2File::get_or_open(&self.fs, inode_num, inode)
        };
        let _io = file.file_vmo.io_lock.lock().await;
        let mut inode = *file.inode_data.read();
        if inode.mode & 0xf000 != 0x8000 || (inode.links_count == 1 && inode.file_acl != 0) {
            return Err(InvocationError::UnsupportedOperation);
        }
        inode.links_count = inode.links_count.checked_sub(1).ok_or(Ext2Error::InvalidMetadata)?;
        inode.ctime = get_realtime().0 as u32;

        self.remove_dir_entry(name).await?;
        *file.inode_data.write() = inode;

        let mut dirty = self.fs.dirty_files.lock();
        if inode.links_count == 0 {
            dirty.remove(&inode_num);
        } else {
            dirty.insert(inode_num, Arc::clone(&file));
        }

        Ok(())
    }

    async fn create_child_file(&self, name: &str, owner: UserID) -> Result<Arc<dyn KernelObject>, InvocationError> {
        validate_child_name(name)?;
        let creator_uid = u16::try_from(owner.0).map_err(|_| InvocationError::InvalidArgument)?;

        let _namespace = self.fs.namespace_lock.lock().await;
        let parent = *self.inode_data.read();
        self.fs.check_directory_writable(&parent)?;
        if self.fs.lookup_in_dir(&parent, name).await?.is_some() {
            return Err(InvocationError::InvalidArgument);
        }

        let new_inode_num = self.fs.allocate_inode(false).await?;

        let current_time = get_realtime().0 as u32;

        // populate diskinode for regular file
        let child_inode = DiskInode {
            mode: 0x81A4, // regular file (0x8000) + permissions (0o644)
            uid: creator_uid,
            size: 0,
            atime: current_time,
            ctime: current_time,
            mtime: current_time,
            dtime: 0,
            gid: 0,
            links_count: 1,
            blocks: 0,
            flags: 0,
            osdl1: 0,
            data: crate::storage::fs::ext2::structs::FileData {
                blocks: crate::storage::fs::ext2::structs::DiskBlockPointers {
                    direct: [0; 12],
                    single_indirect: 0,
                    double_indirect: 0,
                    triple_indirect: 0,
                },
            },
            generation: 0,
            file_acl: 0,
            dir_acl: 0,
            faddr: 0,
            osd2: [0; 12],
        };

        self.publish_child(&parent, name, new_inode_num, child_inode, None).await?;

        let file = Ext2File::get_or_open(&self.fs, new_inode_num, child_inode);
        Ok(Arc::new(FileDescription::new(file)))
    }

    async fn create_child_dir(&self, name: &str, owner: UserID) -> Result<Arc<dyn KernelObject>, InvocationError> {
        validate_child_name(name)?;
        let creator_uid = u16::try_from(owner.0).map_err(|_| InvocationError::InvalidArgument)?;

        let _namespace = self.fs.namespace_lock.lock().await;
        let mut parent = *self.inode_data.read();
        self.fs.check_directory_writable(&parent)?;
        if self.fs.lookup_in_dir(&parent, name).await?.is_some() {
            return Err(InvocationError::InvalidArgument);
        }

        parent.links_count = parent.links_count.checked_add(1).ok_or(Ext2Error::InvalidMetadata)?;
        let block_size = self.fs.metadata_block_size()? as usize;
        let buffer = PhysBuffer::new().ok_or(InvocationError::OutOfMemory)?;
        let new_inode_num = self.fs.allocate_inode(true).await?;

        let block_id = match self.fs.allocate_block().await {
            Ok(block) => block,
            Err(error) => {
                if let Err(cleanup_error) = self.fs.free_inode(new_inode_num, true).await {
                    klogln!("[EXT2] could not release inode {} after block allocation failed: {:?}", new_inode_num, cleanup_error);
                }
                return Err(error.into());
            }
        };

        let bytes = unsafe { slice::from_raw_parts_mut(buffer.virt() as *mut u8, block_size) };
        bytes.fill(0);
        self.write_record(&mut bytes[..12], new_inode_num, 2, b".");
        self.write_record(&mut bytes[12..], self.inode_num, 2, b"..");

        let current_time = get_realtime().0 as u32;

        let child_inode = DiskInode {
            mode: 0x41ED, // directory (0x4000) + permissions (0o755)
            uid: creator_uid,
            size: self.fs.block_size,
            atime: current_time,
            ctime: current_time,
            mtime: current_time,
            dtime: 0,
            gid: 0,
            links_count: 2, // "." plus parent's link
            blocks: self.fs.sectors_per_block,
            flags: 0,
            osdl1: 0,
            data: crate::storage::fs::ext2::structs::FileData {
                blocks: crate::storage::fs::ext2::structs::DiskBlockPointers {
                    direct: {
                        let mut d = [0; 12];
                        d[0] = block_id;
                        d
                    },
                    single_indirect: 0,
                    double_indirect: 0,
                    triple_indirect: 0,
                },
            },
            generation: 0,
            file_acl: 0,
            dir_acl: 0,
            faddr: 0,
            osd2: [0; 12],
        };

        self.publish_child(&parent, name, new_inode_num, child_inode, Some((block_id, &buffer))).await?;

        let new_dir = Arc::new(Ext2Directory { fs: Arc::clone(&self.fs), inode_num: new_inode_num, inode_data: RwLock::new(child_inode) });

        self.fs.active_dirs.lock().insert(new_inode_num, Arc::downgrade(&new_dir));

        Ok(new_dir)
    }
}

fn register_created_object(object: Arc<dyn KernelObject>) -> Result<usize, InvocationError> {
    let rights = allowed_rights(&object)?;
    let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
    Ok(proc.handles.write().insert(object, rights).0)
}

impl Ext2Directory {
    fn write_record(&self, bytes: &mut [u8], inode: u32, file_type: u8, name: &[u8]) {
        const HEADER_SIZE: usize = size_of::<DiskDirHeader>();
        let header = DiskDirHeader {
            inode,
            record_length: bytes.len() as u16,
            name_length: name.len() as u8,
            file_type: if self.fs.directory_file_types { file_type } else { 0 },
        };
        unsafe { ptr::write(bytes.as_mut_ptr().cast::<DiskDirHeader>(), header) };
        bytes[HEADER_SIZE..HEADER_SIZE + name.len()].copy_from_slice(name);
    }

    async fn list_entries(&self) -> Result<Vec<(String, u8)>, Ext2Error> {
        let _namespace = self.fs.namespace_lock.lock().await;
        let inode = *self.inode_data.read();
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let mut entries = Vec::new();
        self.fs.walk_directory::<()>(&inode, &buffer, |record| {
            if record.header.inode != 0 {
                if let Ok(name) = str::from_utf8(record.name) {
                    if !matches!(name, "" | "." | "..") {
                        entries.try_reserve(1).map_err(|_| Ext2Error::OutOfMemory)?;
                        let mut owned = String::new();
                        owned.try_reserve_exact(name.len()).map_err(|_| Ext2Error::OutOfMemory)?;
                        owned.push_str(name);
                        entries.push((owned, record.header.file_type));
                    }
                }
            }
            Ok(None)
        }).await?;
        Ok(entries)
    }

    async fn publish_child(
        &self,
        parent: &DiskInode,
        name: &str,
        inode_num: u32,
        mut inode: DiskInode,
        contents: Option<(u32, &PhysBuffer)>,
    ) -> Result<(), InvocationError> {
        let file_type = if contents.is_some() { 2 } else { 1 };
        let result = async {
            if let Some((block, contents)) = contents {
                self.fs.write_metadata_block(block, contents).await?;
            }
            self.fs.write_inode(inode_num, &inode).await?;
            self.add_dir_entry(parent, name, inode_num, file_type).await
        }.await;

        if result.is_err() {
            inode.links_count = 0;
            self.fs.queue_orphan(inode_num, inode);
        }
        result
    }

    async fn add_dir_entry(
        &self, parent: &DiskInode, name: &str, child_inode_num: u32, file_type: u8,
    ) -> Result<(), InvocationError> {
        const HEADER_SIZE: usize = size_of::<DiskDirHeader>();
        let name = name.as_bytes();
        let needed = (HEADER_SIZE + name.len()).next_multiple_of(4);
        let block_size = self.fs.metadata_block_size()? as usize;
        let buffer = PhysBuffer::new().ok_or(InvocationError::OutOfMemory)?;

        let slot = self.fs.walk_directory(parent, &buffer, |record| {
            let used = if record.header.inode == 0 {
                0
            } else {
                (HEADER_SIZE + record.name.len()).next_multiple_of(4)
            };
            let length = record.header.record_length as usize;
            Ok((length - used >= needed).then_some((record.offset, used, length)))
        }).await?;

        let bytes = unsafe { slice::from_raw_parts_mut(buffer.virt() as *mut u8, block_size) };
        if let Some((block, (offset, used, length))) = slot {
            if used != 0 {
                let previous = unsafe { bytes.as_mut_ptr().add(offset).cast::<DiskDirHeader>() };
                unsafe { (*previous).record_length = used as u16 };
            }
            self.write_record(&mut bytes[offset + used..offset + length], child_inode_num, file_type, name);
            if file_type == 2 {
                self.fs.write_inode_with_blocks(self.inode_num, parent, &[block], |_, cached| {
                    cached.copy_from_slice(bytes);
                }).await?;
                *self.inode_data.write() = *parent;
            } else {
                self.fs.write_metadata_block(block, &buffer).await?;
            }
        } else {
            bytes.fill(0);
            self.write_record(bytes, child_inode_num, file_type, name);
            let next = self.fs.append_directory_block(self.inode_num, parent, &buffer).await?;
            *self.inode_data.write() = next;
        }
        Ok(())
    }

    async fn remove_dir_entry(&self, name: &str) -> Result<(), InvocationError> {
        let inode = *self.inode_data.read();
        self.fs.check_directory_writable(&inode)?;
        let buffer = PhysBuffer::new().ok_or(InvocationError::OutOfMemory)?;
        let (block, (offset, previous, length)) = self.fs.walk_directory(&inode, &buffer, |record| {
            Ok((record.header.inode != 0 && record.name == name.as_bytes()).then_some((
                record.offset, record.previous_offset, record.header.record_length,
            )))
        }).await?.ok_or(InvocationError::PathNotFound)?;

        unsafe {
            let base = buffer.virt() as *mut u8;
            if let Some(previous) = previous {
                (*base.add(previous).cast::<DiskDirHeader>()).record_length += length;
            } else {
                (*base.add(offset).cast::<DiskDirHeader>()).inode = 0;
            }
        }
        self.fs.write_metadata_block(block, &buffer).await?;
        Ok(())
    }
}
