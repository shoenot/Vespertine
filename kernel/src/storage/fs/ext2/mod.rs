use alloc::collections::{BTreeMap, VecDeque};
use alloc::slice;
use alloc::sync::{
    Arc,
    Weak,
};
use alloc::vec::Vec;
use core::ptr;

use crate::executor::async_mutex::AsyncMutex;
use crate::object::invoke::InvocationError;
use crate::sync::{
    Mutex,
    TicketLock,
};
use crate::memory::PhysBuffer;
use crate::storage::blockdev::{
    AsyncBlockDevice,
    BlockCache,
};
use crate::storage::fs::ext2::obj::{
    Ext2Directory,
    Ext2File,
};
use crate::storage::fs::ext2::structs::{
    DiskGroupDesc,
    DiskInode,
    DiskSuperblock,
};

mod directory;
pub mod init;
mod mount;
pub mod obj;
pub mod permissions;
mod reclaim;
pub mod structs;

use reclaim::PointerEdit;

#[derive(Default)]
struct StagedFileBlock {
    data: u32,
    allocated: [u32; 4],
    count: usize,
    link: Option<(u32, usize, u32)>,
}

impl StagedFileBlock {
    fn record_allocation(&mut self, block: u32) {
        self.allocated[self.count] = block;
        self.count += 1;
    }
}

#[derive(Debug)]
pub struct Ext2FileSystem {
    pub partition: Arc<dyn AsyncBlockDevice>,
    pub cache: BlockCache,

    pub block_size: u32,
    pub sectors_per_block: u32,
    blocks_count: u32,
    first_data_block: u32,
    inodes_count: u32,
    first_inode: u32,
    directory_file_types: bool,
    pub inodes_per_group: u32,
    pub blocks_per_group: u32,
    pub inode_size: u32,

    pub bgdt: TicketLock<Vec<DiskGroupDesc>>,
    pub allocation_lock: AsyncMutex<()>,
    namespace_lock: AsyncMutex<()>,
    reclaim_queue: TicketLock<VecDeque<reclaim::ReclaimJob>>,
    pub active_files: Mutex<BTreeMap<u32, Weak<Ext2File>>>,
    pub active_dirs: Mutex<BTreeMap<u32, Weak<Ext2Directory>>>,
    pub dirty_files: Mutex<BTreeMap<u32, Arc<Ext2File>>>,
}

#[derive(Debug)]
pub enum Ext2Error {
    OutOfMemory,
    BlockRead,
    BlockWrite,
    InvalidMetadata,
    UnsupportedDirectoryLayout,
    NoSpace,
}

impl From<Ext2Error> for InvocationError {
    fn from(value: Ext2Error) -> Self {
        match value {
            Ext2Error::OutOfMemory => InvocationError::OutOfMemory,
            Ext2Error::BlockRead | Ext2Error::BlockWrite | Ext2Error::InvalidMetadata => InvocationError::Io,
            Ext2Error::UnsupportedDirectoryLayout => InvocationError::UnsupportedOperation,
            Ext2Error::NoSpace => InvocationError::NoSpace,
        }
    }
}

impl Ext2FileSystem {
    fn metadata_block_size(&self) -> Result<u32, Ext2Error> {
        let block_size = self.block_size;
        if !matches!(block_size, 1024 | 2048 | 4096) {
            return Err(Ext2Error::InvalidMetadata);
        }
        Ok(block_size)
    }

    fn inode_location(&self, inode_num: u32) -> Result<(u32, usize), Ext2Error> {
        let block_size = self.metadata_block_size()?;

        if inode_num == 0 || inode_num > self.inodes_count {
            return Err(Ext2Error::InvalidMetadata);
        }

        let idx = inode_num - 1;
        let group_idx = (idx / self.inodes_per_group) as usize;
        let local_idx = idx % self.inodes_per_group;

        let inode_table = {
            let groups = self.bgdt.lock();
            let group = groups.get(group_idx).ok_or(Ext2Error::InvalidMetadata)?;
            group.inode_table
        };

        if inode_table == 0 {
            return Err(Ext2Error::InvalidMetadata);
        }

        let byte_offset = local_idx
            .checked_mul(self.inode_size)
            .ok_or(Ext2Error::InvalidMetadata)?;
        let block = inode_table
            .checked_add(byte_offset / block_size)
            .ok_or(Ext2Error::InvalidMetadata)?;
        let offset = (byte_offset % block_size) as usize;

        if offset + size_of::<DiskInode>() > block_size as usize {
            return Err(Ext2Error::InvalidMetadata);
        }
        Ok((block, offset))
    }

    pub async fn read_inode(&self, inode_num: u32) -> Result<DiskInode, Ext2Error> {
        let (block, offset) = self.inode_location(inode_num)?;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        self.read_metadata_block(block, &buffer).await?;

        let inode = unsafe {
            let src_ptr = (buffer.virt() as *const u8).add(offset).cast::<DiskInode>();
            ptr::read(src_ptr)
        };
        Ok(inode)
    }

    pub async fn resolve_file_block(&self, inode: &DiskInode, file_block_idx: usize) -> Result<u32, Ext2Error> {
        let (depth, indices) = self.indirect_path(file_block_idx)?;

        if depth == 0 {
            return self.checked_file_pointer(unsafe { inode.data.blocks.direct[file_block_idx] });
        }

        let mut block = Self::get_indirect_root(inode, depth);
        for &index in &indices[..depth] {
            if block == 0 {
                return Ok(0);
            }
            block = self.read_pointer(block, index).await?;
        }
        Ok(block)
    }

    fn checked_file_pointer(&self, block: u32) -> Result<u32, Ext2Error> {
        if block >= self.blocks_count {
            return Err(Ext2Error::InvalidMetadata);
        }
        Ok(block)
    }

    fn add_inode_block(&self, inode: &mut DiskInode) {
        let blocks = inode.blocks;
        inode.blocks = blocks.saturating_add(self.sectors_per_block);
    }

    async fn allocate_zeroed_block(&self) -> Result<u32, Ext2Error> {
        let block_size = self.metadata_block_size()? as usize;
        let page = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let block = self.allocate_block().await?;

        unsafe {
            ptr::write_bytes(page.virt() as *mut u8, 0, block_size);
        }

        if let Err(error) = self.write_metadata_block(block, &page).await {
            self.release_unlinked_block(block).await;
            return Err(error);
        }
        Ok(block)
    }

    async fn release_unlinked_block(&self, block: u32) {
        if let Err(error) = self.free_block(block).await {
            crate::klogln!("[EXT2] deferring release of unlinked block {}: {:?}", block, error);
            self.queue_retired_blocks(alloc::vec![block]);
        }
    }

    async fn read_pointer(&self, block: u32, index: usize) -> Result<u32, Ext2Error> {
        let pointers = self.metadata_block_size()? as usize / size_of::<u32>();
        if block == 0 || index >= pointers {
            return Err(Ext2Error::InvalidMetadata);
        }

        let page = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        self.read_metadata_block(block, &page).await?;

        self.checked_file_pointer(unsafe { ptr::read((page.virt() as *const u32).add(index)) })
    }

    async fn write_pointer(&self, block: u32, index: usize, value: u32) -> Result<(), Ext2Error> {
        let pointers = self.metadata_block_size()? as usize / size_of::<u32>();
        if block == 0 || index >= pointers {
            return Err(Ext2Error::InvalidMetadata);
        }

        let edit = PointerEdit::Set { block, index, value };
        self.cache.update_blocks(&[block as usize], |_, bytes| edit.apply(bytes))
            .await.map_err(|_| Ext2Error::BlockWrite)
    }

    fn indirect_path(&self, file_block_idx: usize) -> Result<(usize, [usize; 3]), Ext2Error> {
        let pointers = self.metadata_block_size()? as usize / size_of::<u32>();
        let double = pointers.checked_mul(pointers).ok_or(Ext2Error::InvalidMetadata)?;
        let triple = double.checked_mul(pointers).ok_or(Ext2Error::InvalidMetadata)?;
        if file_block_idx < 12 {
            return Ok((0, [file_block_idx, 0, 0]));
        }
        let mut remaining = file_block_idx - 12;
        if remaining < pointers {
            return Ok((1, [remaining, 0, 0]));
        }
        remaining -= pointers;
        if remaining < double {
            return Ok((2, [remaining / pointers, remaining % pointers, 0]));
        }
        remaining -= double;
        if remaining < triple {
            return Ok((3, [remaining / double, (remaining % double) / pointers, remaining % pointers]));
        }
        Err(Ext2Error::InvalidMetadata)
    }

    fn get_indirect_root(inode: &DiskInode, depth: usize) -> u32 {
        unsafe {
            match depth {
                1 => inode.data.blocks.single_indirect,
                2 => inode.data.blocks.double_indirect,
                3 => inode.data.blocks.triple_indirect,
                _ => 0,
            }
        }
    }

    fn set_indirect_root(inode: &mut DiskInode, depth: usize, value: u32) {
        match depth {
            1 => inode.data.blocks.single_indirect = value,
            2 => inode.data.blocks.double_indirect = value,
            3 => inode.data.blocks.triple_indirect = value,
            _ => {}
        }
    }

    pub async fn allocate_file_block(&self, inode: &mut DiskInode, file_block_idx: usize) -> Result<u32, Ext2Error> {
        let mut next = *inode;
        let staged = self.stage_file_block(&mut next, file_block_idx, None).await?;
        if let Some((table, index, child)) = staged.link {
            if let Err(error) = self.write_pointer(table, index, child).await {
                self.release_staged_blocks(&staged).await;
                return Err(error);
            }
        }
        *inode = next;
        Ok(staged.data)
    }

    async fn append_directory_block(
        &self,
        inode_num: u32,
        inode: &DiskInode,
        contents: &PhysBuffer,
    ) -> Result<DiskInode, Ext2Error> {
        let index = self.directory_block_count(inode)?;
        let size = inode.size.checked_add(self.block_size).ok_or(Ext2Error::NoSpace)?;
        let mut next = *inode;
        let staged = self.stage_file_block(&mut next, index, Some(contents)).await?;
        if staged.count == 0 {
            return Err(Ext2Error::InvalidMetadata);
        }

        next.size = size;
        let edit = staged.link.map(|(block, index, value)| PointerEdit::Set { block, index, value });
        if let Err(error) = self.write_inode_and_tables(inode_num, &next, edit.as_slice()).await {
            self.release_staged_blocks(&staged).await;
            return Err(error);
        }
        Ok(next)
    }

    /// Build the new branch without changing an existing inode or indirect table.
    async fn stage_file_block(
        &self,
        inode: &mut DiskInode,
        file_block_idx: usize,
        contents: Option<&PhysBuffer>,
    ) -> Result<StagedFileBlock, Ext2Error> {
        let (depth, indices) = self.indirect_path(file_block_idx)?;
        let mut staged = StagedFileBlock::default();
        let mut parent = None;
        let mut missing_tables = &indices[..depth];

        if depth == 0 {
            let existing = unsafe { inode.data.blocks.direct[file_block_idx] };
            if existing != 0 {
                staged.data = self.checked_file_pointer(existing)?;
                return Ok(staged);
            }
        } else {
            let mut table = Self::get_indirect_root(inode, depth);
            if table != 0 {
                for (level, &index) in indices[..depth].iter().enumerate() {
                    let child = self.read_pointer(table, index).await?;
                    if child == 0 {
                        parent = Some((table, index));
                        missing_tables = &indices[level + 1..depth];
                        break;
                    }

                    if level + 1 == depth {
                        staged.data = child;
                        return Ok(staged);
                    }
                    table = child;
                }
            }
        }

        let result = async {
            let data = self.allocate_block().await?;
            staged.data = data;
            staged.record_allocation(data);
            if let Some(contents) = contents {
                self.write_metadata_block(data, contents).await?;
            }

            let mut child = data;
            for &index in missing_tables.iter().rev() {
                let table = self.allocate_zeroed_block().await?;
                staged.record_allocation(table);
                self.write_pointer(table, index, child).await?;
                child = table;
            }

            if depth == 0 {
                unsafe { inode.data.blocks.direct[file_block_idx] = data };
            } else if let Some((table, index)) = parent {
                staged.link = Some((table, index, child));
            } else {
                Self::set_indirect_root(inode, depth, child);
            }

            for _ in 0..staged.count {
                self.add_inode_block(inode);
            }
            Ok(())
        }.await;

        if let Err(error) = result {
            self.release_staged_blocks(&staged).await;
            return Err(error);
        }
        Ok(staged)
    }

    async fn release_staged_blocks(&self, staged: &StagedFileBlock) {
        for &block in staged.allocated[..staged.count].iter().rev() {
            self.release_unlinked_block(block).await;
        }
    }

    pub async fn write_inode(&self, inode_num: u32, inode: &DiskInode) -> Result<(), Ext2Error> {
        self.write_inode_and_tables(inode_num, inode, &[]).await
    }

    async fn read_metadata_block(&self, block: u32, buffer: &PhysBuffer) -> Result<(), Ext2Error> {
        if block >= self.blocks_count {
            return Err(Ext2Error::InvalidMetadata);
        }
        self.cache.read_block(block as usize, buffer.phys() as u64).await
            .map_err(|_| Ext2Error::BlockRead)
    }

    async fn write_metadata_block(&self, block: u32, buffer: &PhysBuffer) -> Result<(), Ext2Error> {
        if block >= self.blocks_count {
            return Err(Ext2Error::InvalidMetadata);
        }
        self.cache.write_block(block as usize, buffer.phys() as u64).await
            .map_err(|_| Ext2Error::BlockWrite)
    }

    async fn claim_group_bit(&self, bitmap_block: u32, entries_per_group: u32, buffer: &PhysBuffer) -> Result<u32, Ext2Error> {
        let block_size = self.metadata_block_size()?;
        if bitmap_block == 0 || entries_per_group == 0 || entries_per_group > block_size * 8 {
            return Err(Ext2Error::InvalidMetadata);
        }

        self.read_metadata_block(bitmap_block, buffer).await?;

        let bitmap = unsafe {
            slice::from_raw_parts_mut(buffer.virt() as *mut u8, block_size as usize)
        };

        for (byte_index, byte) in bitmap.iter_mut().enumerate() {
            if *byte == u8::MAX { continue };

            let bit = byte.trailing_ones();
            let index = byte_index as u32 * 8 + bit;
            if index >= entries_per_group { break };

            *byte |= 1u8 << bit;
            return Ok(index);
        }

        // the caller picked a group whose descriptor advertised free entries.
        Err(Ext2Error::InvalidMetadata)
    }

    // The allocation lock covers the staged bitmap and descriptor. Publish the
    // bitmap and both on-disk counters together so an error can be retried.
    async fn commit_allocation(
        &self,
        group: usize,
        descriptor: DiskGroupDesc,
        bitmap: u32,
        buffer: &PhysBuffer,
        update_superblock: impl Fn(&mut DiskSuperblock) + Send,
    ) -> Result<(), Ext2Error> {
        let block_size = self.metadata_block_size()? as usize;
        let bgdt_start = (self.first_data_block + 1) as usize;
        let descriptor_offset = group * size_of::<DiskGroupDesc>();
        let descriptor_block = bgdt_start + descriptor_offset / block_size;
        let descriptor_offset = descriptor_offset % block_size;

        let superblock = 1024 / block_size;
        let superblock_offset = 1024 % block_size;
        let bitmap = bitmap as usize;
        let bitmap_bytes = unsafe { slice::from_raw_parts(buffer.virt() as *const u8, block_size) };
        self.cache.update_blocks(&[bitmap, descriptor_block, superblock], move |block, bytes| {
            if block == bitmap {
                bytes.copy_from_slice(bitmap_bytes);
            } else if block == descriptor_block {
                unsafe {
                    let dest = bytes.as_mut_ptr().add(descriptor_offset).cast::<DiskGroupDesc>();
                    ptr::write(dest, descriptor);
                }
            } else {
                unsafe {
                    let dest = bytes.as_mut_ptr().add(superblock_offset).cast::<DiskSuperblock>();
                    update_superblock(&mut *dest);
                }
            }
        }).await.map_err(|_| Ext2Error::BlockWrite)?;

        self.bgdt.lock()[group] = descriptor;
        Ok(())
    }

    pub async fn allocate_block(&self) -> Result<u32, Ext2Error> {
        let _guard = self.allocation_lock.lock().await;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let group_count = self.bgdt.lock().len();

        for group in 0..group_count {
            let mut descriptor = self.bgdt.lock()[group];
            if descriptor.free_blocks_count == 0 {
                continue;
            }

            let bitmap = descriptor.block_bitmap;
            let first = self.first_data_block + group as u32 * self.blocks_per_group;
            let entries = self.blocks_per_group.min(self.blocks_count - first);
            let block = first + self.claim_group_bit(bitmap, entries, &buffer).await?;
            if block == 0 {
                return Err(Ext2Error::InvalidMetadata);
            }

            descriptor.free_blocks_count -= 1;
            self.commit_allocation(group, descriptor, bitmap, &buffer, |sb| {
                sb.free_blocks_count = sb.free_blocks_count.saturating_sub(1);
            }).await?;

            return Ok(block);
        }

        Err(Ext2Error::NoSpace)
    }

    pub async fn allocate_inode(&self, is_dir: bool) -> Result<u32, Ext2Error> {
        let _guard = self.allocation_lock.lock().await;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let group_count = self.bgdt.lock().len();

        for group in 0..group_count {
            let mut descriptor = self.bgdt.lock()[group];
            if descriptor.free_inodes_count == 0 {
                continue;
            }

            let bitmap = descriptor.inode_bitmap;
            let index = self.claim_group_bit(bitmap, self.inodes_per_group, &buffer).await?;

            let inode = (group as u32)
                .checked_mul(self.inodes_per_group)
                .and_then(|base| base.checked_add(index))
                .and_then(|index| index.checked_add(1))
                .ok_or(Ext2Error::InvalidMetadata)?;
            if inode < self.first_inode || inode > self.inodes_count {
                return Err(Ext2Error::InvalidMetadata);
            }

            if is_dir {
                descriptor.used_dirs_count = descriptor.used_dirs_count
                    .checked_add(1)
                    .ok_or(Ext2Error::InvalidMetadata)?;
            }

            descriptor.free_inodes_count -= 1;
            self.commit_allocation(group, descriptor, bitmap, &buffer, |sb| {
                sb.free_inodes_count = sb.free_inodes_count.saturating_sub(1);
            }).await?;

            return Ok(inode);
        }

        Err(Ext2Error::NoSpace)
    }

    async fn stage_released_bit(
        &self,
        bitmap_block: u32,
        index: u32,
        buffer: &PhysBuffer,
    ) -> Result<(), Ext2Error> {
        if bitmap_block == 0 || index >= self.metadata_block_size()? * 8 {
            return Err(Ext2Error::InvalidMetadata);
        }

        self.read_metadata_block(bitmap_block, buffer).await?;

        let byte = unsafe { &mut *(buffer.virt() as *mut u8).add((index / 8) as usize) };
        let mask = 1 << (index % 8);
        if *byte & mask == 0 {
            return Err(Ext2Error::InvalidMetadata);
        }
        *byte &= !mask;
        Ok(())
    }

    pub async fn free_inode(&self, inode_num: u32, is_dir: bool) -> Result<(), Ext2Error> {
        if inode_num < self.first_inode || inode_num > self.inodes_count {
            return Err(Ext2Error::InvalidMetadata);
        }

        let _guard = self.allocation_lock.lock().await;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let group = ((inode_num - 1) / self.inodes_per_group) as usize;
        let index = (inode_num - 1) % self.inodes_per_group;

        let mut descriptor = *self.bgdt.lock().get(group).ok_or(Ext2Error::InvalidMetadata)?;
        descriptor.free_inodes_count = descriptor.free_inodes_count
            .checked_add(1)
            .ok_or(Ext2Error::InvalidMetadata)?;
        if is_dir {
            descriptor.used_dirs_count = descriptor.used_dirs_count.saturating_sub(1);
        }
        let bitmap = descriptor.inode_bitmap;
        self.stage_released_bit(bitmap, index, &buffer).await?;
        self.commit_allocation(group, descriptor, bitmap, &buffer, |sb| {
            sb.free_inodes_count = sb.free_inodes_count.saturating_add(1);
        }).await
    }

    pub async fn free_block(&self, block_id: u32) -> Result<(), Ext2Error> {
        if block_id == 0 {
            return Ok(());
        }
        if !(self.first_data_block..self.blocks_count).contains(&block_id) {
            return Err(Ext2Error::InvalidMetadata);
        }

        let _guard = self.allocation_lock.lock().await;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let relative_block = block_id - self.first_data_block;
        let group = (relative_block / self.blocks_per_group) as usize;
        let index = relative_block % self.blocks_per_group;

        let mut descriptor = *self.bgdt.lock().get(group).ok_or(Ext2Error::InvalidMetadata)?;
        descriptor.free_blocks_count = descriptor.free_blocks_count
            .checked_add(1)
            .ok_or(Ext2Error::InvalidMetadata)?;
        let bitmap = descriptor.block_bitmap;
        self.stage_released_bit(bitmap, index, &buffer).await?;
        self.cache.invalidate(block_id as usize).await;
        self.commit_allocation(group, descriptor, bitmap, &buffer, |sb| {
            sb.free_blocks_count = sb.free_blocks_count.saturating_add(1);
        }).await
    }
}
