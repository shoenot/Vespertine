use alloc::boxed::Box;
use alloc::sync::Arc;
use async_trait::async_trait;

use vespertine_abi::{AccessRights, FileOp, Invocation};
use crate::arch::x86_64::task::syscall::{safe_copy_from, safe_copy_to};
use crate::core::asynchronous::async_mutex::AsyncMutex;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::vmo::VmoObject;
use crate::core::object::obj::KernelObject;
use crate::core::sync::{RwLock, TicketLock};
use crate::core::thread::get_current_process;
use crate::memory::vmo::{FileVmo, PagedBackingStore};
use crate::memory::{ALLOCATOR, BlockSize, HHDMOFFSET};
use crate::storage::fs::ext2::Ext2FileSystem;
use crate::storage::fs::ext2::structs::DiskInode;
use crate::storage::fs::{VfsNode, VfsNodeType};

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
            }
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
            self.file_vmo.resize_object(offset + req_len).map_err(|_| InvocationError::OutOfMemory)?;

            let mut inode_write = self.inode_data.write();
            inode_write.size = (offset + req_len) as u32;
        }

        let mut bytes_copied = 0;

        while bytes_copied < req_len {
            let current_offset = offset + bytes_copied;
            let page_offset = (current_offset / 4096) * 4096;
            let block_internal_offset = current_offset % 4096;

            let phys_addr = self.file_vmo.request_page(page_offset).map_err(|_| InvocationError::InvalidPointer)?;
            let page_virt = phys_addr + *HHDMOFFSET;
            let chunk_size = core::cmp::min(4096 - block_internal_offset, req_len - bytes_copied);

            unsafe {
                let dst_ptr = (page_virt as *mut u8).add(block_internal_offset);
                let src_ptr = (buffer_ptr as *const u8).add(bytes_copied);
                if !safe_copy_from(dst_ptr, src_ptr, chunk_size) {
                    return Err(InvocationError::InvalidPointer);
                }
            }

            self.file_vmo.mark_dirty(page_offset).map_err(|_| InvocationError::InvalidPointer)?;

            bytes_copied += chunk_size;
        }
        let inode_ref = self.inode_data.read();
        self.fs.write_inode(self.inode_num, &*inode_ref).await.map_err(|_| InvocationError::InvalidPointer)?;

        let self_arc = {
            let active = self.fs.active_files.lock();
            active.get(&self.inode_num).and_then(|weak| weak.upgrade())
        };
        if let Some(arc) = self_arc {
            self.fs.dirty_files.lock().insert(self.inode_num, arc);
        }

        Ok(bytes_copied)
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
        let is_contiguous =
            (1..blocks_per_page).all(|i| block_ids[i] != 0 && block_ids[i] == (block_ids[0] + i as u32)) && block_ids[0] != 0;

        if is_contiguous {
            let sectors_to_read = blocks_per_page as u32 * self.fs.sectors_per_block;
            let start_sector = block_ids[0] as u64 * self.fs.sectors_per_block as u64;

            let read_fut = self.fs.partition.read_sectors(start_sector, sectors_to_read, dest_phys as u64).map_err(|_| ())?;
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
                    let sector = block_ids[i] as u64 * self.fs.sectors_per_block as u64;
                    let read_fut =
                        self.fs.partition.read_sectors(sector, self.fs.sectors_per_block, dest_blocks_phys as u64).map_err(|_| ())?;

                    read_fut.await.map_err(|_| ())?;
                }
            }
        }

        Ok(read_len)
    }

    async fn write_at_phys(&self, offset: usize, src_phys: usize, len: usize) -> Result<usize, ()> {
        let _guard = self.write_lock.lock().await;

        let block_size = self.fs.block_size as usize;
        let blocks_per_page = len / block_size;
        let start_file_block = offset / block_size;

        for i in 0..blocks_per_page {
            let file_block_idx = start_file_block + i;
            let src_block_phys = src_phys + (i * block_size);

            let mut disk_block_id = {
                let inode_ref = self.inode_data.read();
                self.fs.resolve_file_block(&*inode_ref, file_block_idx).await.unwrap_or(0)
            };

            if disk_block_id == 0 {
                disk_block_id = self.fs.allocate_block().await.map_err(|_| ())?;

                let mut inode_write = self.inode_data.write();
                if file_block_idx < 12 {
                    unsafe {
                        inode_write.data.blocks.direct[file_block_idx] = disk_block_id;
                    }
                } else {
                    let pointers_per_block = (self.fs.block_size / 4) as usize;
                    let blocks_per_double = pointers_per_block * pointers_per_block;
                    let blocks_per_triple = blocks_per_double * pointers_per_block;
                    let remaining = file_block_idx - 12;

                    if remaining < pointers_per_block {
                        // single indirect resolution
                        let mut single_indirect = unsafe { inode_write.data.blocks.single_indirect };
                        if single_indirect == 0 {
                            single_indirect = self.fs.allocate_block().await.map_err(|_| ())?;
                            inode_write.data.blocks.single_indirect = single_indirect;

                            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys == 0 {
                                return Err(());
                            }
                            unsafe {
                                core::ptr::write_bytes((page_phys + *HHDMOFFSET) as *mut u8, 0, block_size);
                            }
                            let sector = single_indirect as u64 * self.fs.sectors_per_block as u64;
                            let write_fut =
                                self.fs.partition.write_sectors(sector, self.fs.sectors_per_block, page_phys as u64).map_err(|_| ())?;
                            write_fut.await.map_err(|_| ())?;
                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                        }

                        let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys == 0 {
                            return Err(());
                        }
                        let sector = single_indirect as u64 * self.fs.sectors_per_block as u64;
                        let read_fut =
                            self.fs.partition.read_sectors(sector, self.fs.sectors_per_block, page_phys as u64).map_err(|_| ())?;
                        read_fut.await.map_err(|_| ())?;

                        unsafe {
                            let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                            core::ptr::write(table_ptr.add(remaining), disk_block_id);
                        }

                        let write_fut =
                            self.fs.partition.write_sectors(sector, self.fs.sectors_per_block, page_phys as u64).map_err(|_| ())?;
                        write_fut.await.map_err(|_| ())?;
                        ALLOCATOR.free(page_phys, BlockSize::Normal);
                    } else if remaining < pointers_per_block + blocks_per_double {
                        // double indirect
                        let doubly_idx = remaining - pointers_per_block;
                        let mut double_indirect = unsafe { inode_write.data.blocks.double_indirect };

                        if double_indirect == 0 {
                            double_indirect = self.fs.allocate_block().await.map_err(|_| ())?;
                            inode_write.data.blocks.double_indirect = double_indirect;

                            // zero initialize l1 table
                            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys == 0 {
                                return Err(());
                            }
                            unsafe {
                                core::ptr::write_bytes((page_phys + *HHDMOFFSET) as *mut u8, 0, block_size);
                            }
                            self.fs.cache.write_block(double_indirect as usize, page_phys as u64).await.map_err(|_| ())?;
                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                        }

                        let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys == 0 {
                            return Err(());
                        }

                        self.fs.read_block(double_indirect, page_phys as u64).await.map_err(|_| ())?;
                        let level1_idx = doubly_idx / pointers_per_block;
                        let level2_idx = doubly_idx % pointers_per_block;

                        let mut single_indirect = unsafe {
                            let table_ptr = (page_phys + *HHDMOFFSET) as *const u32;
                            core::ptr::read(table_ptr.add(level1_idx))
                        };

                        if single_indirect == 0 {
                            single_indirect = self.fs.allocate_block().await.map_err(|_| ())?;
                            unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                core::ptr::write(table_ptr.add(level1_idx), single_indirect);
                            }
                            self.fs.cache.write_block(double_indirect as usize, page_phys as u64).await.map_err(|_| ())?;

                            // zero init l2 table
                            let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys_sub == 0 {
                                ALLOCATOR.free(page_phys, BlockSize::Normal);
                                return Err(());
                            }
                            unsafe {
                                core::ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                            }
                            self.fs.cache.write_block(single_indirect as usize, page_phys_sub as u64).await.map_err(|_| ())?;
                            ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                        }

                        self.fs.read_block(single_indirect, page_phys as u64).await.map_err(|_| ())?;
                        unsafe {
                            let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                            core::ptr::write(table_ptr.add(level2_idx), disk_block_id);
                        }
                        self.fs.cache.write_block(single_indirect as usize, page_phys as u64).await.map_err(|_| ())?;
                        ALLOCATOR.free(page_phys, BlockSize::Normal);
                    } else if remaining < pointers_per_block + blocks_per_double + blocks_per_triple {
                        // triple indirect
                        let triply_idx = remaining - pointers_per_block - blocks_per_double;
                        let mut triple_indirect = unsafe { inode_write.data.blocks.triple_indirect };

                        if triple_indirect == 0 {
                            triple_indirect = self.fs.allocate_block().await.map_err(|_| ())?;
                            inode_write.data.blocks.triple_indirect = triple_indirect;

                            // zero initialize l1 table
                            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys == 0 {
                                return Err(());
                            }
                            unsafe {
                                core::ptr::write_bytes((page_phys + *HHDMOFFSET) as *mut u8, 0, block_size);
                            }
                            self.fs.cache.write_block(triple_indirect as usize, page_phys as u64).await.map_err(|_| ())?;
                            ALLOCATOR.free(page_phys, BlockSize::Normal);
                        }

                        let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
                        if page_phys == 0 {
                            return Err(());
                        }

                        let level1_idx = triply_idx / blocks_per_double;
                        let level2_idx = (triply_idx % blocks_per_double) / pointers_per_block;
                        let level3_idx = (triply_idx % blocks_per_double) % pointers_per_block;

                        self.fs.read_block(triple_indirect, page_phys as u64).await.map_err(|_| ())?;
                        let mut double_indirect = unsafe {
                            let table_ptr = (page_phys + *HHDMOFFSET) as *const u32;
                            core::ptr::read(table_ptr.add(level1_idx))
                        };

                        if double_indirect == 0 {
                            double_indirect = self.fs.allocate_block().await.map_err(|_| ())?;
                            unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                core::ptr::write(table_ptr.add(level1_idx), double_indirect);
                            }
                            self.fs.cache.write_block(triple_indirect as usize, page_phys as u64).await.map_err(|_| ())?;

                            let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys_sub == 0 {
                                ALLOCATOR.free(page_phys, BlockSize::Normal);
                                return Err(());
                            }
                            unsafe {
                                core::ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                            }
                            self.fs.cache.write_block(double_indirect as usize, page_phys_sub as u64).await.map_err(|_| ())?;
                            ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                        }

                        self.fs.read_block(double_indirect, page_phys as u64).await.map_err(|_| ())?;
                        let mut single_indirect = unsafe {
                            let table_ptr = (page_phys + *HHDMOFFSET) as *const u32;
                            core::ptr::read(table_ptr.add(level2_idx))
                        };

                        if single_indirect == 0 {
                            single_indirect = self.fs.allocate_block().await.map_err(|_| ())?;
                            unsafe {
                                let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                                core::ptr::write(table_ptr.add(level2_idx), single_indirect);
                            }
                            self.fs.cache.write_block(double_indirect as usize, page_phys as u64).await.map_err(|_| ())?;

                            let page_phys_sub = ALLOCATOR.alloc(BlockSize::Normal);
                            if page_phys_sub == 0 {
                                ALLOCATOR.free(page_phys, BlockSize::Normal);
                                return Err(());
                            }
                            unsafe {
                                core::ptr::write_bytes((page_phys_sub + *HHDMOFFSET) as *mut u8, 0, block_size);
                            }
                            self.fs.cache.write_block(single_indirect as usize, page_phys_sub as u64).await.map_err(|_| ())?;
                            ALLOCATOR.free(page_phys_sub, BlockSize::Normal);
                        }

                        self.fs.read_block(single_indirect, page_phys as u64).await.map_err(|_| ())?;
                        unsafe {
                            let table_ptr = (page_phys + *HHDMOFFSET) as *mut u32;
                            core::ptr::write(table_ptr.add(level3_idx), disk_block_id);
                        }
                        self.fs.cache.write_block(single_indirect as usize, page_phys as u64).await.map_err(|_| ())?;
                        ALLOCATOR.free(page_phys, BlockSize::Normal);
                    } else {
                        return Err(());
                    }
                }
                inode_write.blocks += self.fs.sectors_per_block;
            }

            // write direct from vmo frame to part (bypass cache)
            let sector = disk_block_id as u64 * self.fs.sectors_per_block as u64;
            let write_fut = self.fs.partition.write_sectors(sector, self.fs.sectors_per_block, src_block_phys as u64).map_err(|_| ())?;

            write_fut.await.map_err(|_| ())?;
        }

        // save metadata
        let inode_ref = self.inode_data.read();
        self.fs.write_inode(self.inode_num, &*inode_ref).await.map_err(|_| ())?;

        Ok(len)
    }

    fn size(&self) -> usize { self.inode_data.read().size as usize }

    fn resize(&self, new_size: usize) -> Result<(), ()> { self.file_vmo.anonymous_vmo.resize_object(new_size) }

    fn node_type(&self) -> VfsNodeType { VfsNodeType::File }
}
