use alloc::boxed::Box;
use alloc::sync::Arc;

use async_trait::async_trait;
use hal::usercopy::{
    safe_copy_from,
    safe_copy_to,
};
use vespertine_abi::{
    AccessRights,
    FileOp,
    FileStat,
    Invocation,
    ObjectType,
};

use crate::executor::async_mutex::AsyncMutex;
use crate::object::invoke::InvocationError;
use crate::object::vmo::{VmoObject, install_private_file_vmo, new_private_file_vmo};
use crate::object::obj::KernelObject;
use crate::security::permissions::FilePermissions;
use crate::sync::RwLock;
use crate::process::current_process;
use crate::memory::vmo::{
    AnonymousVmo, FileVmo, PagedBackingStore,
};
use crate::memory::{
    ALLOCATOR, DIRECT_MAP_OFFSET, PAGE_SIZE_4K, PageSize, PhysBuffer,
};
use crate::storage::fs::ext2::Ext2FileSystem;
use crate::storage::fs::ext2::permissions::file_permissions;
use crate::storage::fs::ext2::structs::DiskInode;
use crate::storage::fs::{
    VfsNode,
    VfsNodeType,
};

#[derive(Debug)]
pub struct Ext2File {
    pub fs: Arc<Ext2FileSystem>,
    pub inode_num: u32,
    pub inode_data: RwLock<DiskInode>,
    pub file_vmo: Arc<FileVmo>,
}

unsafe impl Send for Ext2File {}
unsafe impl Sync for Ext2File {}

#[async_trait]
impl KernelObject for Ext2File {
    fn type_name(&self) -> &'static str { "File" }

    fn object_type(&self) -> ObjectType { ObjectType::File }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset, buffer_ptr, len }) => {
                let bytes_read = self.read_bytes_async(offset, buffer_ptr, len).await?;
                Ok(bytes_read)
            },
            Invocation::File(FileOp::Stat { stat_ptr }) => {
                let inode = self.inode_data.read();

                let stat = FileStat {
                    object_type: ObjectType::File as u32,
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
            },
            Invocation::File(FileOp::GetVmo) => {
                let vmo_obj = Arc::new(VmoObject::new(self.file_vmo.clone()));
                let current_proc = current_process().ok_or(InvocationError::UnsupportedOperation)?;
                let rights = calling_rights & AccessRights::READ;
                let handle_id = current_proc.handles.write().insert(vmo_obj, rights);

                Ok(handle_id.0 as usize)
            },
            Invocation::File(FileOp::Write { offset, buffer_ptr, len }) => {
                let bytes_written = self.write_bytes_async(offset, buffer_ptr, len).await?;
                Ok(bytes_written)
            },
            Invocation::File(FileOp::Seek { .. }) => Err(InvocationError::UnsupportedOperation),
            Invocation::File(FileOp::Truncate { size }) => {
                self.truncate(size).await?;
                Ok(0)
            },
            Invocation::File(FileOp::GetPrivateVmo { offset, len }) => {
                let _guard = self.file_vmo.io_lock.lock().await;
                let file_size = self.inode_data.read().size as usize;
                let backing = new_private_file_vmo(offset, len, file_size)?;

                let page = PageSize::Size4K.bytes();
                let rounded_len = len.div_ceil(page).saturating_mul(page);
                let bytes = rounded_len.min(file_size - offset);

                self.copy_pages_into_vmo_locked(offset, bytes, &backing).await?;
                install_private_file_vmo(backing, calling_rights)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn permissions(&self) -> Option<FilePermissions> {
        let inode = self.inode_data.read();
        Some(file_permissions(inode.uid, inode.mode))
    }

    async fn copy_file_into_vmo(&self, offset: usize, bytes: usize, backing: &AnonymousVmo) -> Result<(), InvocationError> {
        let _guard = self.file_vmo.io_lock.lock().await;
        let file_size = self.inode_data.read().size as usize;
        let page = PageSize::Size4K.bytes();

        if offset % page != 0 || offset.checked_add(bytes).is_none_or(|end| end > file_size) {
            return Err(InvocationError::InvalidArgument);
        }

        self.copy_pages_into_vmo_locked(offset, bytes, backing).await
    }
}

impl Ext2File {
    async fn read_bytes_async(&self, offset: usize, buffer_ptr: usize, req_len: usize) -> Result<usize, InvocationError> {
        let _guard = self.file_vmo.io_lock.lock().await;
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

            let frame = self.file_vmo
                .get_frame_locked(page_offset, file_size)
                .await
                .map_err(|_| InvocationError::InvalidPointer)?;

            let page_virt = frame.virt();
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
        if req_len == 0 { return Ok(0) };

        if offset.checked_add(req_len).is_none_or(|end| end > u32::MAX as usize) {
            return Err(InvocationError::InvalidArgument);
        }
        if buffer_ptr.checked_add(req_len).is_none() {
            return Err(InvocationError::InvalidPointer);
        }

        let _guard = self.file_vmo.io_lock.lock().await;
        let old_size = self.inode_data.read().size as usize;
        let staging = PhysBuffer::new().ok_or(InvocationError::OutOfMemory)?;

        let file = {
            let active = self.fs.active_files.lock();
            active.get(&self.inode_num).and_then(|weak| weak.upgrade())
        }
        .ok_or(InvocationError::InvalidHandle)?;

        let mut written = 0;
        while written < req_len {
            let file_offset = offset + written;
            let in_page = file_offset % PAGE_SIZE_4K;
            let page_offset = file_offset - in_page;
            let count = (req_len - written).min(PAGE_SIZE_4K - in_page);

            let chunk: Result<(), InvocationError> = async {
                let copied = safe_copy_from(staging.virt() as *mut u8, (buffer_ptr + written) as *const u8, count);
                if !copied { return Err(InvocationError::InvalidPointer) };

                let frame = self.file_vmo.get_frame_locked(page_offset, old_size).await
                    .map_err(|_| InvocationError::UnsupportedOperation)?;

                self.file_vmo.mark_dirty(page_offset)
                    .map_err(|_| InvocationError::UnsupportedOperation)?;

                unsafe {
                    core::ptr::copy_nonoverlapping(
                        staging.virt() as *const u8,
                        (frame.virt() + in_page) as *mut u8,
                        count,
                    );
                }

                Ok(())
            }
            .await;

            if let Err(error) = chunk {
                if written == 0 { return Err(error) };
                break;
            }

            let first_chunk = written == 0;
            written += count;
            {
                let mut inode = self.inode_data.write();
                inode.size = inode.size.max((offset + written) as u32);
            }

            if first_chunk {
                self.fs.dirty_files.lock().insert(self.inode_num, Arc::clone(&file));
            }
        }

        let inode = *self.inode_data.read();
        self.fs.write_inode(self.inode_num, &inode).await
            .map_err(|_| InvocationError::UnsupportedOperation)?;

        Ok(written)
    }

    async fn truncate(&self, new_size: usize) -> Result<(), InvocationError> {
        if new_size > u32::MAX as usize {
            return Err(InvocationError::InvalidArgument);
        }

        let _guard = self.file_vmo.io_lock.lock().await;

        let old_size = self.inode_data.read().size as usize;
        if new_size == old_size {
            return Ok(());
        }

        let block_size = self.fs.block_size as usize;

        if new_size < old_size {
            // Zero bytes after the new EOF in the final retained block.
            if new_size % block_size != 0 {
                let block_index = new_size / block_size;
                let block_offset = new_size % block_size;

                let inode = *self.inode_data.read();
                let block_id = self.fs.resolve_file_block(&inode, block_index).await.map_err(|_| InvocationError::UnsupportedOperation)?;

                if block_id != 0 {
                    let page = PhysBuffer::new().ok_or(InvocationError::OutOfMemory)?;
                    let sectors = self.fs.sectors_per_block;
                    let sector = u64::from(block_id) * u64::from(sectors);

                    self.fs.partition.read_sectors(sector, sectors, page.phys() as u64)
                        .map_err(|_| InvocationError::UnsupportedOperation)?
                        .await
                        .map_err(|_| InvocationError::UnsupportedOperation)?;

                    unsafe {
                        (page.virt() as *mut u8).add(block_offset).write_bytes(0, block_size - block_offset);
                    }

                    self.fs.partition.write_sectors(sector, sectors, page.phys() as u64)
                        .map_err(|_| InvocationError::UnsupportedOperation)?
                        .await
                        .map_err(|_| InvocationError::UnsupportedOperation)?;
                }
            }

            let old_blocks = old_size.div_ceil(block_size);
            let new_blocks = new_size.div_ceil(block_size);

            let mut inode = *self.inode_data.read();

            for block_index in new_blocks..old_blocks {
                self.fs.clear_file_block(&mut inode, block_index).await.map_err(|_| InvocationError::UnsupportedOperation)?;
            }

            inode.size = new_size as u32;

            self.fs.write_inode(self.inode_num, &inode).await.map_err(|_| InvocationError::UnsupportedOperation)?;
            *self.inode_data.write() = inode;
        } else {
            let mut inode = *self.inode_data.read();
            inode.size = new_size as u32;

            self.fs.write_inode(self.inode_num, &inode).await.map_err(|_| InvocationError::UnsupportedOperation)?;
            *self.inode_data.write() = inode;
        }

        if new_size < old_size { self.file_vmo.truncate_cached_locked(new_size) };

        Ok(())
    }

    async fn copy_pages_into_vmo_locked(&self, offset: usize, bytes: usize, backing: &AnonymousVmo) -> Result<(), InvocationError> {
        let page = PageSize::Size4K.bytes();

        for dst_offset in (0..bytes).step_by(page) {
            let src_offset = offset + dst_offset;
            let count = (bytes - dst_offset).min(page);
            let frame = backing.get_page(dst_offset)
                .map_err(|_| InvocationError::OutOfMemory)?;
            let dst_virt = frame.virt();

            if let Some(src_frame) = self.file_vmo.cached_frame(src_offset) {
                unsafe {
                    core::ptr::copy_nonoverlapping(src_frame.virt() as *const u8, dst_virt as *mut u8, count);
                }
            } else {
                let read = self.read_at_phys(src_offset, frame.phys(), count).await
                    .map_err(|_| InvocationError::UnsupportedOperation)?;
                if read != count {
                    return Err(InvocationError::UnsupportedOperation);
                }
            }

            unsafe {
                (dst_virt as *mut u8).add(count).write_bytes(0, page - count);
            }
        }

        Ok(())
    }

    pub async fn flush_dirty_pages(&self) -> Result<(), ()> {
        let _guard = self.file_vmo.io_lock.lock().await;
        let size = self.inode_data.read().size as usize;

        for (offset, frame) in self.file_vmo.dirty_frames() {
            let bytes = size.checked_sub(offset).ok_or(())?.min(PAGE_SIZE_4K);
            if bytes == 0 { return Err(()) };

            self.write_page_locked(offset, frame.phys(), bytes).await?;
            self.file_vmo.clear_dirty(offset);
        }

        self.fs.dirty_files.lock().remove(&self.inode_num);
        Ok(())
    }

    async fn write_page_locked(&self, offset: usize, src_phys: usize, bytes: usize) -> Result<(), ()> {
        let block_size = self.fs.block_size as usize;
        let first_block = offset / block_size;
        let block_count = bytes.div_ceil(block_size);
        let sectors = self.fs.sectors_per_block;

        for index in 0..block_count {
            let file_block = first_block + index;
            let mut inode = *self.inode_data.read();
            let mut disk_block = self.fs.resolve_file_block(&inode, file_block).await?;

            if disk_block == 0 {
                disk_block = self.fs.allocate_file_block(&mut inode, file_block).await?;
                *self.inode_data.write() = inode;
            }

            let sector = u64::from(disk_block) * u64::from(sectors);
            let block_phys = src_phys + index * block_size;
            self.fs.partition
                .write_sectors(sector, sectors, block_phys as u64)?
                .await?;
        }

        let inode = *self.inode_data.read();
        self.fs.write_inode(self.inode_num, &inode).await
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
            let inode = *self.inode_data.read();
            for i in 0..blocks_per_page {
                let file_block_idx = start_file_block + i;
                block_ids[i] = self.fs.resolve_file_block(&inode, file_block_idx).await.map_err(|_| ())?;
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
                        let dest_virt = dest_blocks_phys + *DIRECT_MAP_OFFSET;
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

    fn size(&self) -> usize { self.inode_data.read().size as usize }

    fn node_type(&self) -> VfsNodeType { VfsNodeType::File }
}
