use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};

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

use crate::object::invoke::InvocationError;
use crate::object::vmo::{VmoObject, install_private_file_vmo, new_private_file_vmo};
use crate::object::obj::KernelObject;
use crate::security::permissions::FilePermissions;
use crate::storage::blockdev::BlockFlushError;
use crate::sync::RwLock;
use crate::time::get_realtime;
use crate::process::current_process;
use crate::memory::vmo::{
    AnonymousVmo, FilePageWriteback, FileVmo, MappingPage, PagedBackingStore,
};
use crate::memory::{
    DIRECT_MAP_OFFSET, PAGE_SIZE_4K, PageSize, PhysBuffer,
};
use crate::storage::fs::ext2::{Ext2Error, Ext2FileSystem};
use crate::storage::fs::ext2::permissions::file_permissions;
use crate::storage::fs::ext2::structs::DiskInode;
use crate::storage::fs::{
    VfsNode,
    VfsNodeType,
    VfsReadError,
};

#[derive(Debug)]
pub struct Ext2File {
    pub fs: Arc<Ext2FileSystem>,
    pub inode_num: u32,
    pub inode_data: RwLock<DiskInode>,
    pub file_vmo: Arc<FileVmo>,
    self_ref: Weak<Self>,
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
                let vmo_obj = Arc::new(VmoObject::new(self.arc()));
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
            Invocation::File(FileOp::Sync) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                self.sync().await?;
                Ok(0)
            }
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
    pub(super) fn active(fs: &Ext2FileSystem, inode_num: u32) -> Option<Arc<Self>> {
        fs.active_files.lock().get(&inode_num).and_then(Weak::upgrade)
    }

    pub(super) fn get_or_open(fs: &Arc<Ext2FileSystem>, inode_num: u32, inode: DiskInode) -> Arc<Self> {
        let mut active = fs.active_files.lock();
        if let Some(file) = active.get(&inode_num).and_then(Weak::upgrade) {
            return file;
        }

        let file = Arc::new_cyclic(|me: &Weak<Self>| {
            let node = me.clone() as Weak<dyn VfsNode>;
            Self {
                fs: Arc::clone(fs),
                inode_num,
                inode_data: RwLock::new(inode),
                file_vmo: FileVmo::new(node),
                self_ref: me.clone(),
            }
        });
        active.insert(inode_num, Arc::downgrade(&file));
        file
    }

    fn arc(&self) -> Arc<Self> {
        self.self_ref.upgrade().expect("live ext2 file lost its owning Arc")
    }

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

            let frame = self.file_vmo.get_frame_locked(page_offset, file_size).await?;

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
        let inode = *self.inode_data.read();
        let old_size = inode.size as usize;
        let linked = inode.links_count != 0;
        let staging = PhysBuffer::new().ok_or(InvocationError::OutOfMemory)?;

        let mut written = 0;
        while written < req_len {
            let file_offset = offset + written;
            let in_page = file_offset % PAGE_SIZE_4K;
            let page_offset = file_offset - in_page;
            let count = (req_len - written).min(PAGE_SIZE_4K - in_page);

            let chunk: Result<(), InvocationError> = async {
                let copied = safe_copy_from(staging.virt() as *mut u8, (buffer_ptr + written) as *const u8, count);
                if !copied { return Err(InvocationError::InvalidPointer) };

                let frame = self.file_vmo.get_frame_locked(page_offset, old_size).await?;
                self.file_vmo.mark_dirty(page_offset).map_err(|_| InvocationError::Io)?;

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

            if first_chunk && linked {
                self.fs.dirty_files.lock().insert(self.inode_num, self.arc());
            }
        }

        if linked {
            let inode = *self.inode_data.read();
            self.fs.write_inode(self.inode_num, &inode).await?;
        }

        Ok(written)
    }

    async fn truncate(&self, new_size: usize) -> Result<(), InvocationError> {
        let size = u32::try_from(new_size).map_err(|_| InvocationError::InvalidArgument)?;
        let _guard = self.file_vmo.io_lock.lock().await;
        let mut inode = *self.inode_data.read();
        if inode.mode & 0xf000 != 0x8000 || inode.dir_acl != 0 {
            return Err(InvocationError::UnsupportedOperation);
        }
        if size == inode.size {
            return Ok(());
        }

        let now = get_realtime().0 as u32;
        inode.mtime = now;
        inode.ctime = now;

        if size > inode.size {
            inode.size = size;
            self.fs.write_inode(self.inode_num, &inode).await?;
            *self.inode_data.write() = inode;
            return Ok(());
        }

        let plan = self.fs.plan_shrink(inode, size).await?;
        let block_size = self.fs.metadata_block_size()? as usize;
        let dirty_tail = if new_size % block_size != 0
            && self.fs.resolve_file_block(&inode, new_size / block_size).await? != 0
        {
            let page_offset = new_size - new_size % PAGE_SIZE_4K;
            self.file_vmo.get_frame_locked(page_offset, inode.size as usize).await?;
            Some(new_size - page_offset - new_size % block_size)
        } else {
            None
        };

        // Reserve all edited metadata blocks before publishing any pointer change.
        self.fs.write_inode_and_tables(self.inode_num, &plan.inode, &plan.edits).await?;
        *self.inode_data.write() = plan.inode;
        self.file_vmo.truncate_cached_locked(new_size, dirty_tail);
        if dirty_tail.is_some() && plan.inode.links_count != 0 {
            self.fs.dirty_files.lock().insert(self.inode_num, self.arc());
        }
        self.fs.queue_retired_blocks(plan.retired);
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
                let read = self.read_at_phys(src_offset, frame.phys(), count).await?;
                if read != count {
                    return Err(InvocationError::Io);
                }
            }

            unsafe {
                (dst_virt as *mut u8).add(count).write_bytes(0, page - count);
            }
        }

        Ok(())
    }

    pub async fn flush_dirty_pages(&self) -> Result<(), Ext2Error> {
        let _guard = self.file_vmo.io_lock.lock().await;
        self.flush_dirty_pages_locked().await
    }

    async fn sync(&self) -> Result<(), InvocationError> {
        let _guard = self.file_vmo.io_lock.lock().await;
        let inode = *self.inode_data.read();

        if inode.mode & 0xf000 != 0x8000 || inode.links_count == 0 {
            return Err(InvocationError::UnsupportedOperation);
        }

        self.flush_dirty_pages_locked().await?;
        self.fs.cache.flush().await.map_err(|_| InvocationError::Io)?;

        let flush = self.fs.partition.flush().map_err(|error| match error {
            BlockFlushError::Unsupported => InvocationError::UnsupportedOperation,
            BlockFlushError::Io => InvocationError::Io,
        })?;

        flush.await.map_err(|_| InvocationError::Io)
    }

    async fn flush_dirty_pages_locked(&self) -> Result<(), Ext2Error> {
        let inode = *self.inode_data.read();
        if inode.links_count == 0 {
            return Ok(());
        }
        let size = inode.size as usize;

        for (offset, frame, writeback) in self.file_vmo.dirty_frames() {
            let remaining = size.checked_sub(offset)
                .filter(|&bytes| bytes != 0)
                .ok_or(Ext2Error::InvalidMetadata)?;

            match writeback {
                FilePageWriteback::Page => {
                    self.write_page_locked(offset, frame.phys(), remaining.min(PAGE_SIZE_4K)).await?;
                }
                FilePageWriteback::TailBlock { offset: within_page } => {
                    let block_size = self.fs.metadata_block_size()? as usize;
                    self.write_page_locked(offset + within_page, frame.phys() + within_page, block_size).await?;
                }
            }
            self.file_vmo.clear_dirty(offset);
        }

        // Unlink can enqueue a link-count change without any dirty pages.
        let inode = *self.inode_data.read();
        self.fs.write_inode(self.inode_num, &inode).await?;
        self.fs.dirty_files.lock().remove(&self.inode_num);
        Ok(())
    }

    async fn write_page_locked(&self, offset: usize, src_phys: usize, bytes: usize) -> Result<(), Ext2Error> {
        let block_size = self.fs.metadata_block_size()? as usize;
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
                .write_sectors(sector, sectors, block_phys as u64)
                .map_err(|_| Ext2Error::BlockWrite)?
                .await
                .map_err(|_| Ext2Error::BlockWrite)?;
        }

        let inode = *self.inode_data.read();
        self.fs.write_inode(self.inode_num, &inode).await
    }
}

impl Drop for Ext2File {
    fn drop(&mut self) {
        let inode = *self.inode_data.read();
        if inode.links_count == 0 {
            self.fs.queue_orphan(self.inode_num, inode);
        }
    }
}

impl PagedBackingStore for Ext2File {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        self.file_vmo.request_page(offset)
    }

    fn page_for_mapping(&self, offset: usize) -> Result<MappingPage, ()> {
        self.file_vmo.page_for_mapping(offset)
    }
}

#[async_trait]
impl VfsNode for Ext2File {
    async fn read_at_phys(&self, offset: usize, dest_phys: usize, len: usize) -> Result<usize, VfsReadError> {
        if offset % PAGE_SIZE_4K != 0 || len > PAGE_SIZE_4K {
            return Err(VfsReadError::InvalidArgument);
        }

        let file_size = self.inode_data.read().size as usize;
        if offset >= file_size {
            return Ok(0);
        }

        let bytes_available = file_size - offset;
        let read_len = core::cmp::min(bytes_available, len);
        if read_len == 0 {
            return Ok(0);
        }

        let block_size = self.fs.metadata_block_size().map_err(|_| VfsReadError::Io)? as usize;
        let block_count = read_len.div_ceil(block_size);
        let start_file_block = offset / block_size;

        let mut block_ids = [0u32; PAGE_SIZE_4K / 1024];
        let inode = *self.inode_data.read();
        let blocks = &mut block_ids[..block_count];
        for (index, block) in blocks.iter_mut().enumerate() {
            *block = self.fs.resolve_file_block(&inode, start_file_block + index).await
                .map_err(|error| match error {
                    Ext2Error::OutOfMemory => VfsReadError::OutOfMemory,
                    _ => VfsReadError::Io,
                })?;
        }

        let sectors = self.fs.sectors_per_block;
        let is_contiguous = blocks[0] != 0 && blocks.windows(2)
            .all(|pair| pair[0].checked_add(1) == Some(pair[1]));

        if is_contiguous {
            let first_sector = u64::from(blocks[0]) * u64::from(sectors);
            let count = block_count as u32 * sectors;
            self.fs.partition.read_sectors(first_sector, count, dest_phys as u64)
                .map_err(|_| VfsReadError::Io)?
                .await
                .map_err(|_| VfsReadError::Io)?;
        } else {
            for (index, &block) in blocks.iter().enumerate() {
                let phys = dest_phys + index * block_size;
                if block == 0 {
                    unsafe {
                        let virt = phys + *DIRECT_MAP_OFFSET;
                        core::ptr::write_bytes(virt as *mut u8, 0, block_size);
                    }
                } else {
                    let sector = u64::from(block) * u64::from(sectors);
                    self.fs.partition.read_sectors(sector, sectors, phys as u64)
                        .map_err(|_| VfsReadError::Io)?
                        .await
                        .map_err(|_| VfsReadError::Io)?;
                }
            }
        }

        Ok(read_len)
    }

    fn size(&self) -> usize { self.inode_data.read().size as usize }

    fn node_type(&self) -> VfsNodeType { VfsNodeType::File }
}
