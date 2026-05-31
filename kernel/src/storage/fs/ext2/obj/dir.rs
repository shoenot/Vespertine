use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use core::ptr::{self, copy_nonoverlapping};
use async_trait::async_trait;

use vespertine_abi::protocol::{AbiDirEntry, DirEntryType, PacketFlags, PacketHeader, VESPER_MAGIC};
use vespertine_abi::{AccessRights, DirectoryOp, FileOp, Invocation};
use crate::core::asynchronous::async_mutex::AsyncMutex;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::directory::Filename;
use crate::core::object::obj::KernelObject;
use crate::core::sync::{RwLock, TicketLock};
use crate::core::thread::get_current_process;
use crate::memory::vmo::FileVmo;
use crate::memory::{ALLOCATOR, BlockSize, HHDMOFFSET};
use crate::storage::fs::ext2::Ext2FileSystem;
use crate::storage::fs::ext2::structs::{DiskDirHeader, DiskInode};
use crate::storage::fs::VfsNode;

use super::file::Ext2File;

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
                            inode_data: RwLock::new(child_inode_data),
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
            },

            Invocation::Directory(DirectoryOp::Link { name, name_len, handle_id }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }

                let filename = Filename::new(name as *const u8, name_len)?;

                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let obj_arc = {
                    let table = proc.proc_handles.read();
                    let entry = table.get(&handle_id).ok_or(InvocationError::InvalidHandle)?;
                    entry.object.clone()
                };

                let (child_inode_num, file_type) = if obj_arc.type_name() == "File" {
                    let file_ref = unsafe {
                        let raw_fat = Arc::into_raw(obj_arc.clone());
                        let raw_thin = raw_fat as *const () as *const Ext2File;
                        Arc::from_raw(raw_thin)
                    };
                    (file_ref.inode_num, 1u8)
                } else if obj_arc.type_name() == "Directory" {
                    let dir_ref = unsafe {
                        let raw_fat = Arc::into_raw(obj_arc.clone());
                        let raw_thin = raw_fat as *const () as *const Ext2Directory;
                        Arc::from_raw(raw_thin)
                    };
                    (dir_ref.inode_num, 2u8)
                } else {
                    return Err(InvocationError::UnsupportedOperation);
                };

                self.add_dir_entry(&*filename.name, child_inode_num, file_type)
                    .await
                    .map_err(|_| InvocationError::UnsupportedOperation)?;

                let mut child_inode = self.fs.read_inode(child_inode_num).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                child_inode.links_count += 1;
                self.fs.write_inode(child_inode_num, &child_inode).await.map_err(|_| InvocationError::UnsupportedOperation)?;

                Ok(0)
            }
            Invocation::Directory(DirectoryOp::Unlink { name, name_len }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }

                let filename = Filename::new(name as *const u8, name_len)?;

                let child_inode_num = self
                    .fs
                    .lookup_in_dir(&self.inode_data.read(), &*filename.name)
                    .await
                    .map_err(|_| InvocationError::PathNotFound)?
                    .ok_or(InvocationError::PathNotFound)?;

                let mut child_inode = self.fs.read_inode(child_inode_num).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                let is_dir = (child_inode.mode & 0xF000) == 0x4000;

                self.remove_dir_entry(&*filename.name)
                    .await
                    .map_err(|_| InvocationError::UnsupportedOperation)?;

                if child_inode.links_count > 0 {
                    child_inode.links_count -= 1;
                }

                if child_inode.links_count == 0 {
                    let block_size = self.fs.block_size as usize;
                    let total_blocks = child_inode.size as usize / block_size;
                    for block_idx in 0..total_blocks {
                        let block_id = self.fs.resolve_file_block(&child_inode, block_idx).await.unwrap_or(0);
                        if block_id != 0 {
                            self.fs.free_block(block_id).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                        }
                    }

                    let single_indirect = unsafe { child_inode.data.blocks.single_indirect };
                    if single_indirect != 0 {
                        self.fs.free_block(single_indirect).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                    }

                    let double_indirect = unsafe { child_inode.data.blocks.double_indirect };
                    if double_indirect != 0 {
                        let mut sub_blocks = alloc::vec::Vec::new();
                        let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys != 0 {
                            let page_virt = page_phys + *HHDMOFFSET;
                            if self.fs.read_block(double_indirect, page_phys as u64).await.is_ok() {
                                let pointers_per_block = (self.fs.block_size / 4) as usize;
                                unsafe {
                                    let table_ptr = page_virt as *const u32;
                                    for i in 0..pointers_per_block {
                                        let sub_block = core::ptr::read(table_ptr.add(i));
                                        if sub_block != 0 {
                                            sub_blocks.push(sub_block);
                                        }
                                    }
                                }
                            }
                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                        }
                        for sub_block in sub_blocks {
                            let _ = self.fs.free_block(sub_block).await;
                        }
                        self.fs.free_block(double_indirect).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                    }

                    let triple_indirect = unsafe { child_inode.data.blocks.triple_indirect };
                    if triple_indirect != 0 {
                        let mut d_blocks = alloc::vec::Vec::new();
                        let mut s_blocks = alloc::vec::Vec::new();
                        
                        let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys != 0 {
                            let page_virt = page_phys + *HHDMOFFSET;
                            if self.fs.read_block(triple_indirect, page_phys as u64).await.is_ok() {
                                let pointers_per_block = (self.fs.block_size / 4) as usize;
                                unsafe {
                                    let table_ptr = page_virt as *const u32;
                                    for i in 0..pointers_per_block {
                                        let d_block = core::ptr::read(table_ptr.add(i));
                                        if d_block != 0 {
                                            d_blocks.push(d_block);
                                        }
                                    }
                                }
                            }
                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                        }

                        for &d_block in &d_blocks {
                            let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys_sub != 0 {
                                let page_virt_sub = page_phys_sub + *HHDMOFFSET;
                                if self.fs.read_block(d_block, page_phys_sub as u64).await.is_ok() {
                                    let pointers_per_block = (self.fs.block_size / 4) as usize;
                                    unsafe {
                                        let sub_table_ptr = page_virt_sub as *const u32;
                                        for j in 0..pointers_per_block {
                                            let s_block = core::ptr::read(sub_table_ptr.add(j));
                                            if s_block != 0 {
                                                s_blocks.push(s_block);
                                            }
                                        }
                                    }
                                }
                                ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                            }
                        }

                        for s_block in s_blocks {
                            let _ = self.fs.free_block(s_block).await;
                        }
                        for d_block in d_blocks {
                            let _ = self.fs.free_block(d_block).await;
                        }
                        self.fs.free_block(triple_indirect).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                    }

                    self.fs.free_inode(child_inode_num, is_dir).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                } else {
                    self.fs.write_inode(child_inode_num, &child_inode).await.map_err(|_| InvocationError::UnsupportedOperation)?;
                }

                Ok(0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl Ext2Directory {
    pub async fn add_dir_entry(&self, name: &str, child_inode_num: u32, file_type: u8) -> Result<(), ()> {
        let name_bytes = name.as_bytes();
        if name_bytes.len() > 254 || name_bytes.is_empty() {
            return Err(());
        }

        let needed_len = (8 + name_bytes.len() + 3) & !3;
        let block_size = self.fs.block_size as usize;

        let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
        if page_phys == 0 {
            return Err(());
        }
        let page_virt = page_phys + *HHDMOFFSET;

        let total_blocks = self.inode_data.read().size as usize / block_size;
        for block_idx in 0..total_blocks {
            let block_id = {
                let inode_ref = self.inode_data.read();
                self.fs.resolve_file_block(&*inode_ref, block_idx).await.unwrap_or(0)
            };
            if block_id == 0 {
                continue;
            }

            self.fs.read_block(block_id, page_phys as u64).await?;

            let mut offset = 0;
            while offset < block_size {
                unsafe {
                    let entry_ptr = (page_virt as *mut u8).add(offset) as *mut DiskDirHeader;
                    let rec_len = (*entry_ptr).record_length as usize;
                    let name_len = (*entry_ptr).name_length as usize;

                    if rec_len == 0 {
                        break;
                    }

                    if offset + rec_len == block_size {
                        let last_used_len = (8 + name_len + 3) & !3;
                        let padding = rec_len - last_used_len;

                        if padding >= needed_len {
                            (*entry_ptr).record_length = last_used_len as u16;

                            let new_entry_ptr = (page_virt as *mut u8).add(offset + last_used_len) as *mut DiskDirHeader;
                            ptr::write(
                                new_entry_ptr,
                                DiskDirHeader {
                                    inode: child_inode_num,
                                    record_length: padding as u16,
                                    name_length: name_bytes.len() as u8,
                                    file_type,
                                },
                            );

                            let new_name_ptr = (new_entry_ptr as *mut u8).add(8);
                            copy_nonoverlapping(name_bytes.as_ptr(), new_name_ptr, name_bytes.len());

                            self.fs.cache.write_block(block_id as usize, page_phys as u64).await?;
                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                            return Ok(());
                        }
                    }
                    offset += rec_len;
                }
            }
        }

        let new_block_id = match self.fs.allocate_block().await {
            Ok(id) => id,
            Err(_) => {
                ALLOCATOR.free(page_phys, BlockSize::Normal);
                return Err(());
            }
        };

        unsafe {
            ptr::write_bytes(page_virt as *mut u8, 0, block_size);
            let new_entry_ptr = page_virt as *mut DiskDirHeader;
            ptr::write(
                new_entry_ptr,
                DiskDirHeader {
                    inode: child_inode_num,
                    record_length: block_size as u16,
                    name_length: name_bytes.len() as u8,
                    file_type,
                },
            );

            let new_name_ptr = (new_entry_ptr as *mut u8).add(8);
            copy_nonoverlapping(name_bytes.as_ptr(), new_name_ptr, name_bytes.len());
        }

        let sector = new_block_id as u64 * self.fs.sectors_per_block as u64;
        let write_result = {
            let write_fut = self.fs.partition.write_sectors(sector, self.fs.sectors_per_block, page_phys as u64);
            match write_fut {
                Ok(fut) => fut.await,
                Err(_) => Err(()),
            }
        };
        if write_result.is_err() {
            ALLOCATOR.free(page_phys, BlockSize::Normal);
            return Err(());
        }

        let new_logical_idx = total_blocks;
        let mut inode_write = self.inode_data.write();

        let map_result = {
            if new_logical_idx < 12 {
                unsafe {
                    inode_write.data.blocks.direct[new_logical_idx] = new_block_id;
                }
                Ok(())
            } else {
                let pointers_per_block = (self.fs.block_size / 4) as usize;
                let blocks_per_double = pointers_per_block * pointers_per_block;
                let blocks_per_triple = blocks_per_double * pointers_per_block;
                let remaining = new_logical_idx - 12;

                if remaining < pointers_per_block {
                    // single indirection
                    let mut single_indirect = unsafe { inode_write.data.blocks.single_indirect };
                    if single_indirect == 0 {
                        single_indirect = match self.fs.allocate_block().await {
                            Ok(id) => id,
                            Err(_) => return Err(()),
                        };
                        inode_write.data.blocks.single_indirect = single_indirect;

                        unsafe {
                            ptr::write_bytes(page_virt as *mut u8, 0, block_size);
                        }
                        let s = single_indirect as u64 * self.fs.sectors_per_block as u64;
                        self.fs.partition.write_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    }

                    let s = single_indirect as u64 * self.fs.sectors_per_block as u64;
                    self.fs.partition.read_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    unsafe {
                        let table_ptr = (page_virt as *mut u8) as *mut u32;
                        ptr::write(table_ptr.add(remaining), new_block_id);
                    }
                    self.fs.partition.write_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    Ok(())
                } else if remaining < pointers_per_block + blocks_per_double {
                    // double indirection
                    let doubly_idx = remaining - pointers_per_block;
                    let mut double_indirect = unsafe { inode_write.data.blocks.double_indirect };
                    if double_indirect == 0 {
                        double_indirect = match self.fs.allocate_block().await {
                            Ok(id) => id,
                            Err(_) => return Err(()),
                        };
                        inode_write.data.blocks.double_indirect = double_indirect;

                        unsafe {
                            ptr::write_bytes(page_virt as *mut u8, 0, block_size);
                        }
                        let s = double_indirect as u64 * self.fs.sectors_per_block as u64;
                        self.fs.partition.write_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    }

                    let s = double_indirect as u64 * self.fs.sectors_per_block as u64;
                    self.fs.partition.read_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    let level1_idx = doubly_idx / pointers_per_block;
                    let level2_idx = doubly_idx % pointers_per_block;

                    let mut single_indirect = unsafe {
                        let table_ptr = (page_virt as *mut u8) as *mut u32;
                        ptr::read(table_ptr.add(level1_idx))
                    };

                    if single_indirect == 0 {
                        let new_sub_block = match self.fs.allocate_block().await {
                            Ok(id) => id,
                            Err(_) => return Err(()),
                        };
                        single_indirect = new_sub_block;

                        unsafe {
                            let table_ptr = (page_virt as *mut u8) as *mut u32;
                            ptr::write(table_ptr.add(level1_idx), single_indirect);
                        }
                        self.fs.partition.write_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;

                        let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys_sub == 0 { return Err(()); }
                        unsafe {
                            ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                        }
                        let sub_s = single_indirect as u64 * self.fs.sectors_per_block as u64;
                        self.fs.partition.write_sectors(sub_s, self.fs.sectors_per_block, page_phys_sub as u64)?.await?;
                        ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                    }

                    let sub_s = single_indirect as u64 * self.fs.sectors_per_block as u64;
                    self.fs.partition.read_sectors(sub_s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    unsafe {
                        let table2_ptr = (page_virt as *mut u8) as *mut u32;
                        ptr::write(table2_ptr.add(level2_idx), new_block_id);
                    }
                    self.fs.partition.write_sectors(sub_s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    Ok(())
                } else if remaining < pointers_per_block + blocks_per_double + blocks_per_triple {
                    // triple indirection
                    let triply_idx = remaining - pointers_per_block - blocks_per_double;
                    let mut triple_indirect = unsafe { inode_write.data.blocks.triple_indirect };
                    if triple_indirect == 0 {
                        triple_indirect = match self.fs.allocate_block().await {
                            Ok(id) => id,
                            Err(_) => return Err(()),
                        };
                        inode_write.data.blocks.triple_indirect = triple_indirect;

                        unsafe {
                            ptr::write_bytes(page_virt as *mut u8, 0, block_size);
                        }
                        let s = triple_indirect as u64 * self.fs.sectors_per_block as u64;
                        self.fs.partition.write_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    }

                    let s = triple_indirect as u64 * self.fs.sectors_per_block as u64;
                    self.fs.partition.read_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    let level1_idx = triply_idx / blocks_per_double;
                    let level2_idx = (triply_idx % blocks_per_double) / pointers_per_block;
                    let level3_idx = (triply_idx % blocks_per_double) % pointers_per_block;

                    let mut double_indirect = unsafe {
                        let table_ptr = (page_virt as *mut u8) as *mut u32;
                        ptr::read(table_ptr.add(level1_idx))
                    };

                    if double_indirect == 0 {
                        let new_sub_block = match self.fs.allocate_block().await {
                            Ok(id) => id,
                            Err(_) => return Err(()),
                        };
                        double_indirect = new_sub_block;

                        unsafe {
                            let table_ptr = (page_virt as *mut u8) as *mut u32;
                            ptr::write(table_ptr.add(level1_idx), double_indirect);
                        }
                        self.fs.partition.write_sectors(s, self.fs.sectors_per_block, page_phys as u64)?.await?;

                        let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys_sub == 0 { return Err(()); }
                        unsafe {
                            ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                        }
                        let sub_s = double_indirect as u64 * self.fs.sectors_per_block as u64;
                        self.fs.partition.write_sectors(sub_s, self.fs.sectors_per_block, page_phys_sub as u64)?.await?;
                        ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                    }

                    let sub_s = double_indirect as u64 * self.fs.sectors_per_block as u64;
                    self.fs.partition.read_sectors(sub_s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    let mut single_indirect = unsafe {
                        let table2_ptr = (page_virt as *mut u8) as *mut u32;
                        ptr::read(table2_ptr.add(level2_idx))
                    };

                    if single_indirect == 0 {
                        let new_sub_block = match self.fs.allocate_block().await {
                            Ok(id) => id,
                            Err(_) => return Err(()),
                        };
                        single_indirect = new_sub_block;

                        unsafe {
                            let table2_ptr = (page_virt as *mut u8) as *mut u32;
                            ptr::write(table2_ptr.add(level2_idx), single_indirect);
                        }
                        self.fs.partition.write_sectors(sub_s, self.fs.sectors_per_block, page_phys as u64)?.await?;

                        let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys_sub == 0 { return Err(()); }
                        unsafe {
                            ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                        }
                        let sub2_s = single_indirect as u64 * self.fs.sectors_per_block as u64;
                        self.fs.partition.write_sectors(sub2_s, self.fs.sectors_per_block, page_phys_sub as u64)?.await?;
                        ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                    }

                    let sub2_s = single_indirect as u64 * self.fs.sectors_per_block as u64;
                    self.fs.partition.read_sectors(sub2_s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    unsafe {
                        let table3_ptr = (page_virt as *mut u8) as *mut u32;
                        ptr::write(table3_ptr.add(level3_idx), new_block_id);
                    }
                    self.fs.partition.write_sectors(sub2_s, self.fs.sectors_per_block, page_phys as u64)?.await?;
                    Ok(())
                } else {
                    Err(())
                }
            }
        };

        if map_result.is_err() {
            ALLOCATOR.free(page_phys, BlockSize::Normal);
            return Err(());
        }

        inode_write.size += block_size as u32;
        inode_write.blocks += self.fs.sectors_per_block;

        let num = self.inode_num;
        let save_result = self.fs.write_inode(num, &*inode_write).await;

        ALLOCATOR.free(page_phys, BlockSize::Normal);
        save_result
    }

    pub async fn remove_dir_entry(&self, name: &str) -> Result<(), ()> {
        let name_bytes = name.as_bytes();
        if name_bytes.is_empty() || name_bytes.len() > 254 {
            return Err(());
        }

        let block_size = self.fs.block_size as usize;
        let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
        if page_phys == 0 {
            return Err(());
        }
        let page_virt = page_phys + *HHDMOFFSET;

        let total_blocks = self.inode_data.read().size as usize / block_size;
        for block_idx in 0..total_blocks {
            let block_id = {
                let inode_ref = self.inode_data.read();
                self.fs.resolve_file_block(&*inode_ref, block_idx).await.unwrap_or(0)
            };
            if block_id == 0 {
                continue;
            }

            if self.fs.read_block(block_id, page_phys as u64).await.is_err() {
                ALLOCATOR.free(page_phys, BlockSize::Normal);
                return Err(());
            }

            let mut offset = 0;
            let mut prev_entry_ptr: Option<*mut DiskDirHeader> = None;

            while offset < block_size {
                unsafe {
                    let entry_ptr = (page_virt as *mut u8).add(offset) as *mut DiskDirHeader;
                    let rec_len = (*entry_ptr).record_length as usize;
                    let name_len = (*entry_ptr).name_length as usize;
                    let inode_id = (*entry_ptr).inode;

                    if rec_len == 0 {
                        break;
                    }

                    if inode_id != 0 && name_len == name_bytes.len() {
                        let name_ptr = (entry_ptr as *const u8).add(8);
                        let name_slice = core::slice::from_raw_parts(name_ptr, name_len);
                        if name_slice == name_bytes {
                            if let Some(prev) = prev_entry_ptr {
                                (*prev).record_length += rec_len as u16;
                            } else {
                                (*entry_ptr).inode = 0;
                            }

                            if self.fs.cache.write_block(block_id as usize, page_phys as u64).await.is_err() {
                                ALLOCATOR.free(page_phys, BlockSize::Normal);
                                return Err(());
                            }

                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                            return Ok(());
                        }
                    }

                    prev_entry_ptr = Some(entry_ptr);
                    offset += rec_len;
                }
            }
        }

        ALLOCATOR.free(page_phys, BlockSize::Normal);
        Err(())
    }
}
