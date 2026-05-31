use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
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
    Invocation,
};

use crate::arch::x86_64::task::syscall::{safe_copy_from, safe_copy_to};
use crate::core::asynchronous::async_mutex::AsyncMutex;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::directory::Filename;
use crate::core::object::models::vmo::VmoObject;
use crate::core::object::obj::KernelObject;
use crate::core::sync::{RwLock, TicketLock};
use crate::core::thread::get_current_process;
use crate::storage::fs::{VfsNode, VfsNodeType};
use crate::storage::fs::ext2::Ext2FileSystem;
use crate::storage::fs::ext2::structs::{
    DiskDirHeader,
    DiskInode,
};
use crate::memory::vmo::{
    FileVmo,
    PagedBackingStore,
};
use crate::memory::{
    ALLOCATOR,
    BlockSize,
    HHDMOFFSET,
};

#[derive(Debug)]
pub struct Ext2File {
    pub fs: Arc<Ext2FileSystem>,
    pub inode_num: u32,
    pub inode_data: RwLock<DiskInode>,
    pub file_vmo: Arc<FileVmo>,
    pub offset: TicketLock<usize>,
    pub write_lock: AsyncMutex<()>,
}

unsafe impl Send for Ext2File {}
unsafe impl Sync for Ext2File {}

#[async_trait]
impl KernelObject for Ext2File {
    fn type_name(&self) -> &'static str { "File" }

    async fn invoke(&self, invocation: Invocation, _rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset: _, buffer_ptr, len }) => {
                let mut offset_guard = self.offset.lock();
                let current_offset = *offset_guard;
                let bytes_read = self.read_bytes_async(current_offset, buffer_ptr, len).await?;

                *offset_guard += bytes_read;
                Ok(bytes_read)
            }
            Invocation::File(FileOp::Stat) => Ok(self.inode_data.read().size as usize),
            Invocation::File(FileOp::GetVmo) => {
                let vmo_obj = Arc::new(VmoObject::new(self.file_vmo.clone()));
                let current_proc = get_current_process().ok_or(InvocationError::UnsupportedOperation)?;
                let handle_id = current_proc.proc_handles.write().insert(vmo_obj, AccessRights::all());

                Ok(handle_id.0 as usize)
            },
            Invocation::File(FileOp::Write { offset: _, buffer_ptr, len }) => {
                let mut offset_guard = self.offset.lock();
                let current_offset = *offset_guard;
                let bytes_written = self.write_bytes_async(current_offset, buffer_ptr, len).await?;

                *offset_guard += bytes_written;
                Ok(bytes_written)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl Ext2File {
    async fn read_bytes_async(&self, offset: usize, buffer_ptr: usize, req_len: usize) -> Result<usize, InvocationError> {
        let file_size = self.inode_data.read().size as usize;
        if offset >= file_size {
            return Ok(0);
        };

        let bytes_available = file_size - offset;
        let read_len = core::cmp::min(bytes_available, req_len);
        if read_len == 0 {
            return Ok(0);
        }

        let mut bytes_copied = 0;

        while bytes_copied < read_len {
            let current_file_offset = offset + bytes_copied;
            let page_offset = (current_file_offset / 4096) * 4096;
            let block_internal_offset = current_file_offset % 4096;

            let phys_addr = self.file_vmo.request_page(page_offset).map_err(|_| InvocationError::InvalidPointer)?;

            let page_virt = phys_addr + *HHDMOFFSET;
            let chunk_size = core::cmp::min(4096 - block_internal_offset, read_len - bytes_copied);

            unsafe {
                let src_ptr = (page_virt as *const u8).add(block_internal_offset);
                let dst_ptr = (buffer_ptr as *mut u8).add(bytes_copied);

                if !safe_copy_to(dst_ptr, src_ptr, chunk_size) {
                    return Err(InvocationError::InvalidPointer);
                }
            }
            bytes_copied += chunk_size;
        }
        Ok(bytes_copied)
    }

    pub async fn write_bytes_async(&self, offset: usize, buffer_ptr: usize, req_len: usize) -> Result<usize, InvocationError> {
        let _guard = self.write_lock.lock().await;

        if req_len == 0 {
            return Ok(0);
        }

        let file_size = self.inode_data.read().size as usize;

        // resize vmo if writing past eof
        if offset + req_len > file_size {
            self.file_vmo.resize_object(offset + req_len)
                .map_err(|_| InvocationError::OutOfMemory)?;

            let mut inode_write = self.inode_data.write();
            inode_write.size = (offset + req_len) as u32;
        }

        let mut bytes_copied = 0;
        let block_size = self.fs.block_size as usize;
        let blocks_per_page = 4096 / block_size;

        while bytes_copied < req_len {
            let current_offset = offset + bytes_copied;
            let page_offset = (current_offset / 4096) * 4096;
            let block_internal_offset = current_offset % 4096;

            let phys_addr = self.file_vmo.request_page(page_offset)
                .map_err(|_| InvocationError::InvalidPointer)?;
            let page_virt = phys_addr + *HHDMOFFSET;
            let chunk_size = core::cmp::min(4096 - block_internal_offset, req_len - bytes_copied);

            unsafe {
                let dst_ptr = (page_virt as *mut u8).add(block_internal_offset);
                let src_ptr = (buffer_ptr as *const u8).add(bytes_copied);
                if !safe_copy_from(dst_ptr, src_ptr, chunk_size) {
                    return Err(InvocationError::InvalidPointer);
                }
            }

            let start_file_block = page_offset / block_size;
            for i in 0..blocks_per_page {
                let file_block_idx = start_file_block + i;
                let src_block_phys = phys_addr + (i * block_size);

                let block_start_offset = i * block_size;
                let block_end_offset = (i + 1) * block_size;
                let write_start_offset = block_internal_offset;
                let write_end_offset = block_internal_offset + chunk_size;

                let is_affected = core::cmp::max(block_start_offset, write_start_offset)
                                < core::cmp::min(block_end_offset, write_end_offset);

                if !is_affected {
                    continue;
                }

                let mut disk_block_id = {
                    let inode_ref = self.inode_data.read();
                    self.fs.resolve_file_block(&*inode_ref, file_block_idx).await
                        .map_err(|_| InvocationError::InvalidPointer)?
                };

                // block allocation trigger
                if disk_block_id == 0 {
                    disk_block_id = self.fs.allocate_block().await
                        .map_err(|_| InvocationError::OutOfMemory)?;

                    let mut inode_write = self.inode_data.write();
                    if file_block_idx < 12 {
                        unsafe { inode_write.data.blocks.direct[file_block_idx] = disk_block_id; }
                    } else {
                        let pointers_per_block = (self.fs.block_size / 4) as usize;
                        let blocks_per_double = pointers_per_block * pointers_per_block;
                        let blocks_per_triple = blocks_per_double * pointers_per_block;
                        let remaining = file_block_idx - 12;

                        if remaining < pointers_per_block {
                            // single redirect
                            let mut single_indirect = unsafe { inode_write.data.blocks.single_indirect };

                            if single_indirect == 0 {
                                single_indirect = self.fs.allocate_block().await.map_err(|_| InvocationError::OutOfMemory)?;
                                inode_write.data.blocks.single_indirect = single_indirect;

                                // zero initialize
                                let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                                if page_phys == 0 { return Err(InvocationError::OutOfMemory); }
                                unsafe {
                                    core::ptr::write_bytes((page_phys + *HHDMOFFSET) as *mut u8, 0, block_size);
                                }
                                self.fs.cache.write_block(single_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                                ALLOCATOR.free(page_phys, BlockSize::Normal);
                            }

                            // load block, write ptr, save back
                            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys == 0 { return Err(InvocationError::OutOfMemory); }
                            self.fs.read_block(single_indirect, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;

                            unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                core::ptr::write(table_ptr.add(remaining), disk_block_id);
                            }
                            self.fs.cache.write_block(single_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            ALLOCATOR.free(page_phys, BlockSize::Normal);

                        } else if remaining < pointers_per_block + blocks_per_double {
                            // double indirect
                            let doubly_idx = remaining - pointers_per_block;
                            let mut double_indirect = unsafe { inode_write.data.blocks.double_indirect };

                            if double_indirect == 0 {
                                double_indirect = self.fs.allocate_block().await.map_err(|_| InvocationError::OutOfMemory)?;
                                inode_write.data.blocks.double_indirect = double_indirect;

                                // zero initialize l1 table
                                let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                                if page_phys == 0 { return Err(InvocationError::OutOfMemory); }
                                unsafe {
                                    core::ptr::write_bytes((page_phys + *HHDMOFFSET) as *mut u8, 0, block_size);
                                }
                                self.fs.cache.write_block(double_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                                ALLOCATOR.free(page_phys, BlockSize::Normal);
                            }

                            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys == 0 { return Err(InvocationError::OutOfMemory); }

                            self.fs.read_block(double_indirect, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            let level1_idx = doubly_idx / pointers_per_block;
                            let level2_idx = doubly_idx % pointers_per_block;

                            let mut single_indirect = unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *const u32;
                                core::ptr::read(table_ptr.add(level1_idx))
                            };

                            if single_indirect == 0 {
                                single_indirect = self.fs.allocate_block().await.map_err(|_| InvocationError::OutOfMemory)?;
                                unsafe {
                                    let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                    core::ptr::write(table_ptr.add(level1_idx), single_indirect);
                                }
                                self.fs.cache.write_block(double_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;

                                // zero init l2 table
                                let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                                if page_phys_sub == 0 {
                                    ALLOCATOR.free(page_phys, BlockSize::Normal);
                                    return Err(InvocationError::OutOfMemory);
                                }
                                unsafe {
                                    core::ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                                }
                                self.fs.cache.write_block(single_indirect as usize, page_phys_sub as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                                ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                            }

                            self.fs.read_block(single_indirect, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                core::ptr::write(table_ptr.add(level2_idx), disk_block_id);
                            }
                            self.fs.cache.write_block(single_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            ALLOCATOR.free(page_phys, BlockSize::Normal);

                        } else if remaining < pointers_per_block + blocks_per_double + blocks_per_triple {
                            // triple indirect
                            let triply_idx = remaining - pointers_per_block - blocks_per_double;
                            let mut triple_indirect = unsafe { inode_write.data.blocks.triple_indirect };

                            if triple_indirect == 0 {
                                triple_indirect = self.fs.allocate_block().await.map_err(|_| InvocationError::OutOfMemory)?;
                                inode_write.data.blocks.triple_indirect = triple_indirect;

                                // zero initialize l1 table
                                let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                                if page_phys == 0 { return Err(InvocationError::OutOfMemory); }
                                unsafe {
                                    core::ptr::write_bytes((page_phys + *HHDMOFFSET) as *mut u8, 0, block_size);
                                }
                                self.fs.cache.write_block(triple_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                                ALLOCATOR.free(page_phys, BlockSize::Normal);
                            }

                            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys == 0 { return Err(InvocationError::OutOfMemory); }

                            let level1_idx = triply_idx / blocks_per_double;
                            let level2_idx = (triply_idx % blocks_per_double) / pointers_per_block;
                            let level3_idx = (triply_idx % blocks_per_double) % pointers_per_block;

                            self.fs.read_block(triple_indirect, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            let mut double_indirect = unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *const u32;
                                core::ptr::read(table_ptr.add(level1_idx))
                            };

                            if double_indirect == 0 {
                                double_indirect = self.fs.allocate_block().await.map_err(|_| InvocationError::OutOfMemory)?;
                                unsafe {
                                    let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                    core::ptr::write(table_ptr.add(level1_idx), double_indirect);
                                }
                                self.fs.cache.write_block(triple_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;

                                let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                                if page_phys_sub == 0 {
                                    ALLOCATOR.free(page_phys, BlockSize::Normal);
                                    return Err(InvocationError::OutOfMemory);
                                }
                                unsafe {
                                    core::ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                                }
                                self.fs.cache.write_block(double_indirect as usize, page_phys_sub as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                                ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                            }

                            self.fs.read_block(double_indirect, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            let mut single_indirect = unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *const u32;
                                core::ptr::read(table_ptr.add(level2_idx))
                            };

                            if single_indirect == 0 {
                                single_indirect = self.fs.allocate_block().await.map_err(|_| InvocationError::OutOfMemory)?;
                                unsafe {
                                    let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                    core::ptr::write(table_ptr.add(level2_idx), single_indirect);
                                }
                                self.fs.cache.write_block(double_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;

                                let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                                if page_phys_sub == 0 {
                                    ALLOCATOR.free(page_phys, BlockSize::Normal);
                                    return Err(InvocationError::OutOfMemory);
                                }
                                unsafe {
                                    core::ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                                }
                                self.fs.cache.write_block(single_indirect as usize, page_phys_sub as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                                ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                            }

                            self.fs.read_block(single_indirect, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                core::ptr::write(table_ptr.add(level3_idx), disk_block_id);
                            }
                            self.fs.cache.write_block(single_indirect as usize, page_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                        } else {
                            return Err(InvocationError::UnsupportedOperation);
                        }
                    }
                    inode_write.blocks += self.fs.sectors_per_block;
                }
                self.fs.cache.write_block(disk_block_id as usize, src_block_phys as u64).await.map_err(|_| InvocationError::InvalidPointer)?;
            }
            bytes_copied += chunk_size;
        }
        let inode_ref = self.inode_data.read();
        self.fs.write_inode(self.inode_num, &*inode_ref).await.map_err(|_| InvocationError::InvalidPointer)?;

        drop(inode_ref);

        // flush cache
        self.fs.cache.flush().await.map_err(|_| InvocationError::InvalidPointer)?;

        Ok(bytes_copied)
    }
}

#[derive(Debug)]
pub struct Ext2Directory {
    pub fs: Arc<Ext2FileSystem>,
    pub inode_num: u32,
    pub inode_data: RwLock<DiskInode>,
}

#[async_trait]
impl KernelObject for Ext2Directory {
    fn type_name(&self) -> &'static str { "Directory" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;

                let child_inode_id = self
                    .fs
                    .lookup_in_dir(&self.inode_data.read(), &*filename.name)
                    .await
                    .map_err(|_| InvocationError::PathNotFound)?
                    .ok_or(InvocationError::PathNotFound)?;

                let child_inode_data = self.fs.read_inode(child_inode_id).await.map_err(|_| InvocationError::PathNotFound)?;

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
                            inode_data: RwLock::new(child_inode_data) 
                        });
                        dirs.insert(child_inode_id, Arc::downgrade(&new_dir));
                        new_dir
                    }
                } else {
                    let mut files = self.fs.active_files.lock();
                    let mut cached = None;
                    if let Some(weak_file) = files.get(&child_inode_id) {
                        cached = weak_file.upgrade();
                    }
                    if let Some(arc_file) = cached {
                        arc_file
                    } else {
                        // new_cyclic passes a weak ptr to the ext2file being built
                        let new_file = Arc::new_cyclic(|me| {
                            let weak_node = me.clone() as Weak<dyn VfsNode>;

                            Ext2File {
                                fs: Arc::clone(&self.fs),
                                inode_num: child_inode_id,
                                inode_data: RwLock::new(child_inode_data.clone()),
                                file_vmo: FileVmo::new(child_inode_data.size as usize, weak_node),
                                offset: TicketLock::new(0),
                                write_lock: AsyncMutex::new(()),
                            }
                        });
                        files.insert(child_inode_id, Arc::downgrade(&new_file));
                        new_file
                    }
                };

                let rights = AccessRights(calling_rights.0 & (AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE).0);

                let handle_id =
                    get_current_process().ok_or(InvocationError::InvalidHandle)?.proc_handles.write().insert(target_object, rights);

                Ok(handle_id.0)
            }
            Invocation::Directory(DirectoryOp::List { offset: _, sink }) => {
                let mut entries = alloc::vec::Vec::new();

                let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                if page_phys == 0 {
                    return Err(InvocationError::OutOfMemory);
                }
                let page_virt = page_phys + *HHDMOFFSET;

                for direct_idx in 0..12 {
                    let block_id = unsafe { self.inode_data.read().data.blocks.direct[direct_idx] };
                    if block_id == 0 {
                        continue;
                    };

                    if self.fs.read_block(block_id, page_phys as u64).await.is_err() {
                        ALLOCATOR.free(page_phys, BlockSize::Normal);
                        return Err(InvocationError::InvalidPointer);
                    }

                    let mut offset = 0;
                    while offset < self.fs.block_size as usize {
                        unsafe {
                            let entry_ptr = (page_virt as *const u8).add(offset) as *const DiskDirHeader;
                            let inode_id = (*entry_ptr).inode;
                            let rec_len = (*entry_ptr).record_length as usize;
                            let name_len = (*entry_ptr).name_length as usize;

                            if rec_len == 0 {
                                break;
                            }

                            if inode_id != 0 && name_len > 0 && offset + 8 + name_len <= self.fs.block_size as usize {
                                let name_ptr = (entry_ptr as *const u8).add(8);
                                let name_slice = core::slice::from_raw_parts(name_ptr, name_len);

                                if let Ok(entry_name) = core::str::from_utf8(name_slice) {
                                    if entry_name != "." && entry_name != ".." {
                                        entries.push((alloc::string::ToString::to_string(entry_name), (*entry_ptr).file_type));
                                    }
                                }
                            }
                            offset += rec_len;
                        }
                    }
                }
                ALLOCATOR.free(page_phys, BlockSize::Normal);

                // resolve sink socket
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let sink_obj = proc.proc_handles.read().resolve(sink, AccessRights::WRITE)?;

                crate::core::asynchronous::Executor::new().spawn(async move {
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

                        let op = FileOp::Write { offset: 0, buffer_ptr: buffer.as_mut_ptr() as usize, len: buffer.len() };
                        if sink_obj.invoke(Invocation::File(op), AccessRights::WRITE).await.is_err() {
                            break;
                        }
                    }
                });

                Ok(0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

#[async_trait]
impl VfsNode for Ext2File {
    async fn read_at_phys(&self, offset: usize, dest_phys: usize, len: usize) -> Result<usize, ()> {
        let file_size = self.inode_data.read().size as usize;
        if offset >= file_size {
            return Ok(0);
        }

        let bytes_available = file_size - offset;
        let read_len = core::cmp::min(bytes_available, len);
        if read_len == 0 {
            return Ok(0);
        }

        let block_size = self.fs.block_size as usize;
        let blocks_per_page = 4096 / block_size;
        let start_file_block = offset / block_size;

        let mut block_ids = [0u32; 4];
        {
            let inode_guard = self.inode_data.read();
            for i in 0..blocks_per_page {
                let file_block_idx = start_file_block + i;
                block_ids[i] = self.fs.resolve_file_block(&*inode_guard, file_block_idx).await.map_err(|_| ())?;
            }
        }

        // block fusion 
        let is_contiguous = (1..blocks_per_page).all(|i| {
        block_ids[i] != 0 && block_ids[i] == (block_ids[0] + i as u32)
        }) && block_ids[0] != 0;

        if is_contiguous {
            let sectors_to_read = blocks_per_page as u32 * self.fs.sectors_per_block;
            let start_sector = block_ids[0] as u64 * self.fs.sectors_per_block as u64;

            let read_fut = self.fs.partition
                .read_sectors(start_sector, sectors_to_read, dest_phys as u64)
                .map_err(|_| ())?;
            read_fut.await.map_err(|_| ())?;
        } else {
            for i in 0..blocks_per_page {
                let dest_blocks_phys = dest_phys + (i * block_size);
                if block_ids[i] == 0 {
                    unsafe {
                        let dest_virt = dest_blocks_phys + *HHDMOFFSET;
                        core::ptr::write_bytes(dest_virt as *mut u8, 0, block_size);
                    }
                } else {
                    self.fs.read_block(block_ids[i], dest_blocks_phys as u64).await.map_err(|_| ())?;
                }
            }
        }

        Ok(read_len)
    }

    async fn write_at_phys(&self, offset: usize, src_phys: usize, len: usize) -> Result<usize, ()> {
        let src_virt = src_phys + *HHDMOFFSET;
        self.write_bytes_async(offset, src_virt, len).await.map_err(|_| ())
    }

    fn size(&self) -> usize {
        self.inode_data.read().size as usize
    }

    fn resize(&self, new_size: usize) -> Result<(), ()> {
        self.file_vmo.resize_object(new_size)
    }

    fn node_type(&self) -> VfsNodeType {
        VfsNodeType::File
    }
}
