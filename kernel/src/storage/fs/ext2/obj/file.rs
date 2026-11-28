use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};

use alloc::vec::Vec;
use async_trait::async_trait;
use hal::usercopy::{
    USER_SPACE_LIMIT, copy_from_user, copy_mapped_bytes, copy_to_user, safe_copy_from, safe_copy_to
};
use vespertine_abi::{
    AccessRights, FileOp, FileStat, HandleID, Invocation, IoCompletion, ObjectType, SysError
};

use crate::executor::Executor;
use crate::object::help::RightsWrapper;
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::object::io_queue::{CompletionSlot, IoQueue};
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
    DIRECT_MAP_OFFSET, Frame4K, PAGE_SIZE_4K, PageSize, PhysBuffer
};
use crate::storage::fs::ext2::{Ext2Error, Ext2FileSystem};
use crate::storage::fs::ext2::permissions::file_permissions;
use crate::storage::fs::ext2::structs::DiskInode;
use crate::storage::fs::{
    VfsNode,
    VfsNodeType,
    VfsReadError,
};

const MAX_ASYNC_IO: usize = 64 * 1024;

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

fn prepare_io(
    queue: HandleID,
    buffer: HandleID,
    offset: usize,
    len: usize,
    buffer_rights: AccessRights,
) -> Result<(CompletionSlot, IoBufferRange), InvocationError> {
    let process = current_process().ok_or(InvocationError::InvalidHandle)?;
    let (queue, buffer) = {
        let handles = process.handles.read();
        (
            handles.resolve(queue, AccessRights::WRITE)?,
            handles.resolve(buffer, buffer_rights)?,
        )
    };

    let queue = queue.as_any().downcast_ref::<IoQueue>().ok_or(InvocationError::InvalidHandle)?;
    let buffer = buffer.as_any().downcast_ref::<VmoObject>().ok_or(InvocationError::InvalidHandle)?;

    let slot = queue.reserve_completion()?;
    let range = IoBufferRange::new(Arc::clone(&buffer.vmo), offset, len)?;
    Ok((slot, range))
}

#[async_trait]
impl KernelObject for Ext2File {
    fn type_name(&self) -> &'static str { "File" }

    fn object_type(&self) -> ObjectType { ObjectType::File }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights, ctx: InvokeContext<'_>) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset, buffer_ptr, len }) => {
                self.read_bytes_async(offset, buffer_ptr, len, ctx).await
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
                self.write_bytes_async(offset, buffer_ptr, len, ctx).await
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
            },
            Invocation::File(FileOp::SubmitReadAt { queue, file_offset, buffer, buffer_offset, len, cookie }) => {
                self.submit_read_at(queue, file_offset, buffer, buffer_offset, len, cookie)
            }
            Invocation::File(FileOp::SubmitWriteAt { queue, file_offset, buffer, buffer_offset, len, cookie }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.submit_write_at(queue, file_offset, buffer, buffer_offset, len, cookie)
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

    async fn read_chunks(
        &self,
        offset: usize,
        req_len: usize,
        ctx: InvokeContext<'_>,
        mut copy_chunk: impl FnMut(usize, usize, usize) -> Result<(), InvocationError> + Send,
    ) -> Result<usize, InvocationError> {
        let cache = &self.file_vmo;
        let mut copied = 0;
        let mut limit = req_len;

        'read: while copied < limit {
            let next = offset.checked_add(copied)
                .ok_or(InvocationError::InvalidArgument)?;
            let next_page = next - next % PAGE_SIZE_4K;

            let io = match cache.io_lock.try_lock() {
                Some(io) => io,
                None => {
                    if let Some(fill) = cache.pending_fill(next_page) {
                        ctx.wait(fill.wait()).await?;
                        continue 'read;
                    }

                    cache.io_lock.lock().await
                }
            };

            let file_size = self.inode_data.read().size as usize;
            limit = limit.min(file_size.saturating_sub(offset));

            while copied < limit {
                let pos = offset + copied;
                let within_page = pos % PAGE_SIZE_4K;
                let page_offset = pos - within_page;

                let Some(frame) = cache.cached_frame(page_offset) else {
                    let fill = cache.start_or_join_fill_locked(page_offset)?;
                    drop(io);
                    ctx.wait(fill.wait()).await?;
                    continue 'read;
                };

                let count = (PAGE_SIZE_4K - within_page).min(limit - copied);
                copy_chunk(copied, frame.virt() + within_page, count)?;
                copied += count;
            }
        }

        Ok(copied)
    }

    async fn read_bytes_async(&self, offset: usize, buffer_ptr: usize, len: usize, ctx: InvokeContext<'_>) -> Result<usize, InvocationError> {
        self.read_chunks(offset, len, ctx, |copied, src, count| {
            let dst = buffer_ptr.checked_add(copied) .ok_or(InvocationError::InvalidPointer)?;
            let end = dst.checked_add(count) .ok_or(InvocationError::InvalidPointer)?;

            let success = if ctx.uses_user_pointers() {
                if dst == 0 || end > USER_SPACE_LIMIT {
                    return Err(InvocationError::InvalidPointer);
                }
                copy_to_user(dst as *mut u8, src as *const u8, count)
            } else {
                safe_copy_to(dst as *mut u8, src as *const u8, count)
            };

            if !success { return Err(InvocationError::InvalidPointer) };
            Ok(())
        }).await
    }

    fn submit_read_at(
        &self,
        queue_handle: HandleID,
        file_offset: usize,
        buffer_handle: HandleID,
        buffer_offset: usize,
        len: usize,
        cookie: usize,
    ) -> Result<usize, InvocationError> {
        if len == 0 || len > MAX_ASYNC_IO || file_offset.checked_add(len).is_none() {
            return Err(InvocationError::InvalidArgument);
        }

        let (slot, destination) = prepare_io(queue_handle, buffer_handle, buffer_offset, len, AccessRights::WRITE)?;
        let file = self.arc();

        Executor::new().spawn(async move {
            let mut bytes = 0;
            let result = file.read_chunks(
                file_offset,
                len,
                InvokeContext::kernel(),
                |copied, src, count| {
                    destination.copy_from_cache(copied, src, count);
                    bytes = copied + count;
                    Ok(())
                },
            ).await;

            let status = match result {
                Ok(_) => SysError::Success,
                Err(error) => SysError::from(error),
            };

            // end the request's access to the destination before publishing
            drop(destination);
            drop(file);
            slot.complete(IoCompletion { cookie, bytes, status: status as usize });
        });

        Ok(0)
    }

    async fn write_bytes_async(
        &self,
        offset: usize,
        buffer_ptr: usize,
        len: usize,
        ctx: InvokeContext<'_>,
    ) -> Result<usize, InvocationError> {
        if len == 0 { return Ok(0) };

        validate_write_range(offset, len)?;
        buffer_ptr.checked_add(len).ok_or(InvocationError::InvalidPointer)?;

        let source = WriteSource::Pointer {
            address: buffer_ptr,
            staging: PhysBuffer::new().ok_or(InvocationError::OutOfMemory)?,
            user_pointers: ctx.uses_user_pointers(),
        };
        let outcome = self.write_chunks(offset, len, source).await;

        if let Some(error) = outcome.inode_error {
            return Err(error);
        }
        match (outcome.bytes, outcome.chunk_error) {
            (0, Some(error)) => Err(error),
            (bytes, _) => Ok(bytes),
        }
    }

    async fn write_chunks(
        &self,
        offset: usize,
        len: usize,
        source: WriteSource<'_>,
    ) -> WriteOutcome {
        let _io = self.file_vmo.io_lock.lock().await;
        let inode = *self.inode_data.read();
        let old_size = inode.size as usize;
        let linked = inode.links_count != 0;
        let mut outcome = WriteOutcome::default();

        while outcome.bytes < len {
            let written = outcome.bytes;
            let pos = offset + written;
            let in_page = pos % PAGE_SIZE_4K;
            let page_offset = pos - in_page;
            let count = (len - written).min(PAGE_SIZE_4K - in_page);

            let chunk = async {
                source.stage(written, count)?;
                let frame = self.file_vmo.get_frame_locked(page_offset, old_size).await?;
                self.file_vmo.mark_dirty(page_offset).map_err(|_| InvocationError::Io)?;
                source.copy_into_cache(written, frame.virt() + in_page, count);
                Ok::<(), InvocationError>(())
            }.await;

            if let Err(error) = chunk {
                outcome.chunk_error = Some(error);
                break;
            }

            outcome.bytes += count;
            {
                let mut inode = self.inode_data.write();
                inode.size = inode.size.max((offset + outcome.bytes) as u32);
            }

            if written == 0 && linked {
                self.fs.dirty_files.lock().insert(self.inode_num, self.arc());
            }
        }

        if linked && outcome.bytes != 0 {
            let inode = *self.inode_data.read();
            outcome.inode_error = self.fs.write_inode(self.inode_num, &inode)
                .await.err().map(InvocationError::from);
        }

        outcome
    }

    fn submit_write_at(
        &self,
        queue: HandleID,
        file_offset: usize,
        buffer: HandleID,
        buffer_offset: usize,
        len: usize,
        cookie: usize,
    ) -> Result<usize, InvocationError> {
        if len == 0 || len > MAX_ASYNC_IO { return Err(InvocationError::InvalidArgument) };
        validate_write_range(file_offset, len)?;

        let (slot, source) = prepare_io(queue, buffer, buffer_offset, len, AccessRights::READ)?;
        let file = self.arc();

        Executor::new().spawn(async move {
            let outcome = file.write_chunks(file_offset, len, WriteSource::Vmo(&source)).await;
            let status = outcome.inode_error.or(outcome.chunk_error)
                .map_or(SysError::Success, SysError::from);

            drop(source);
            drop(file);
            slot.complete(IoCompletion {
                cookie,
                bytes: outcome.bytes,
                status: status as usize,
            });
        });

        Ok(0)
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

struct IoBufferRange {
    _backing: Arc<dyn PagedBackingStore>,
    frames: Vec<Arc<Frame4K>>,
    first_byte: usize,
}

impl IoBufferRange {
    fn new(backing: Arc<dyn PagedBackingStore>, offset: usize, len: usize) -> Result<Self, InvocationError> {
        let vmo = backing.as_anonymous().ok_or(InvocationError::UnsupportedOperation)?;
        let end = offset.checked_add(len).ok_or(InvocationError::InvalidArgument)?;

        if len == 0 || end > vmo.len() { return Err(InvocationError::InvalidArgument) };

        let first_page = offset - offset % PAGE_SIZE_4K;
        let page_count = (end - first_page).div_ceil(PAGE_SIZE_4K);
        let mut frames = Vec::new();
        frames.try_reserve_exact(page_count).map_err(|_| InvocationError::OutOfMemory)?;

        for page in 0..page_count {
            let frame = vmo.get_page(first_page + page * PAGE_SIZE_4K).map_err(|_| InvocationError::OutOfMemory)?;
            frames.push(frame);
        }

        Ok(Self { _backing: backing, frames, first_byte: offset % PAGE_SIZE_4K })
    }

    fn copy_from_cache(&self, offset: usize, src: usize, len: usize) {
        self.for_each_part(offset, len, |addr, copied, count| unsafe {
            copy_mapped_bytes((src + copied) as *const u8, addr as *mut u8, count);
        });
    }

    fn copy_to_cache(&self, offset: usize, dst: usize, len: usize) {
        self.for_each_part(offset, len, |addr, copied, count| unsafe {
            copy_mapped_bytes(addr as *const u8, (dst + copied) as *mut u8, count);
        });
    }

    fn for_each_part(&self, offset: usize, len: usize, mut visit: impl FnMut(usize, usize, usize)) {
        let start = self.first_byte + offset;
        let mut copied = 0;

        while copied < len {
            let pos = start + copied;
            let within_page = pos % PAGE_SIZE_4K;
            let frame = &self.frames[pos / PAGE_SIZE_4K];
            let count = (len - copied).min(PAGE_SIZE_4K - within_page);

            visit(frame.virt() + within_page, copied, count);
            copied += count;
        }
    }
}

#[derive(Default)]
struct WriteOutcome {
    bytes: usize,
    chunk_error: Option<InvocationError>,
    inode_error: Option<InvocationError>,
}

enum WriteSource<'a> {
    Pointer {
        address: usize,
        staging: PhysBuffer,
        user_pointers: bool,
    },
    Vmo(&'a IoBufferRange),
}

impl WriteSource<'_> {
    fn stage(&self, offset: usize, count: usize) -> Result<(), InvocationError> {
        let Self::Pointer { address, staging, user_pointers } = self else {
            return Ok(());
        };

        // the blocking adapter checks address + len before entering the loop.
        let src = address + offset;
        let dst = staging.virt() as *mut u8;
        let copied = if *user_pointers {
            if src == 0 || src + count > USER_SPACE_LIMIT {
                return Err(InvocationError::InvalidPointer);
            }
            copy_from_user(dst, src as *const u8, count)
        } else {
            safe_copy_from(dst, src as *const u8, count)
        };

        if copied { Ok(()) } else { Err(InvocationError::InvalidPointer) }
    }

    fn copy_into_cache(&self, offset: usize, dst: usize, count: usize) {
        match self {
            Self::Pointer { staging, .. } => unsafe {
                core::ptr::copy_nonoverlapping(
                    staging.virt() as *const u8,
                    dst as *mut u8,
                    count,
                );
            },
            Self::Vmo(range) => range.copy_to_cache(offset, dst, count),
        }
    }
}

fn validate_write_range(offset: usize, len: usize) -> Result<(), InvocationError> {
    if offset.checked_add(len).is_none_or(|end| end > u32::MAX as usize) {
        return Err(InvocationError::InvalidArgument);
    }
    Ok(())
}
