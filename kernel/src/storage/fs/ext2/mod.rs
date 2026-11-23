use alloc::collections::BTreeMap;
use alloc::slice;
use alloc::sync::{
    Arc,
    Weak,
};
use alloc::vec::Vec;
use core::{
    ptr,
    str,
};

use crate::executor::async_mutex::AsyncMutex;
use crate::object::invoke::InvocationError;
use crate::sync::{
    Mutex,
    TicketLock,
};
use crate::memory::{
    ALLOCATOR, DIRECT_MAP_OFFSET, PageSize, PhysBuffer, calculate_order,
};
use crate::storage::blockdev::{
    AsyncBlockDevice,
    BlockCache,
};
use crate::storage::fs::ext2::obj::{
    Ext2Directory,
    Ext2File,
};
use crate::storage::fs::ext2::structs::{
    DiskDirHeader,
    DiskGroupDesc,
    DiskInode,
    DiskSuperblock,
};

pub mod init;
pub mod obj;
pub mod permissions;
pub mod structs;

#[derive(Debug)]
pub struct Ext2FileSystem {
    pub partition: Arc<dyn AsyncBlockDevice>,
    pub cache: BlockCache,

    pub block_size: u32,
    pub sectors_per_block: u32,
    pub inodes_per_group: u32,
    pub blocks_per_group: u32,
    pub inode_size: u32,

    pub bgdt: TicketLock<Vec<DiskGroupDesc>>,
    pub allocation_lock: AsyncMutex<()>,
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
    pub async fn mount(partition: Arc<dyn AsyncBlockDevice>) -> Result<Self, ()> {
        let buffer = PhysBuffer::new().ok_or(())?;

        // read the ext2 superblock, which sits at 1024 bytes from part start
        let sb_future = partition.read_sectors(2, 2, buffer.phys() as u64)?;
        sb_future.await?;

        let sb = unsafe { &*(buffer.virt() as *const DiskSuperblock) };

        if sb.magic != 0xEF53 { return Err(()); }

        let block_size = 1024 << sb.log_block_size;
        let cache = BlockCache::new(partition.clone(), block_size as usize, 512);
        let sectors_per_block = block_size / 512;

        let inode_size = if sb.rev_level >= 1 { sb.inode_size } else { 128 } as u32;

        let total_blocks = sb.blocks_count;
        let num_groups = (total_blocks + sb.blocks_per_group - 1) / sb.blocks_per_group;
        let bgdt_bytes = num_groups as usize * size_of::<DiskGroupDesc>();
        let bgdt_blocks = (bgdt_bytes as u32 * block_size - 1) / block_size;
        let bgdt_sectors = bgdt_blocks * sectors_per_block;

        let bgdt_start_block = if block_size == 1024 { 2 } else { 1 };
        let bgdt_start_sector = bgdt_start_block as u64 * sectors_per_block as u64;

        let bgdt_alloc_bytes = bgdt_sectors as usize * 512 + 4095;
        let bgdt_alloc_order = calculate_order(bgdt_alloc_bytes);
        let bgdt_buf_phys = match ALLOCATOR.alloc_order(bgdt_alloc_order) {
            Some(a) => a,
            None => return Err(()),
        };
        let bgdt_buf_virt = bgdt_buf_phys + *DIRECT_MAP_OFFSET;

        let bgdt_future = partition.read_sectors(bgdt_start_sector, bgdt_sectors, bgdt_buf_phys as u64)?;
        bgdt_future.await?;

        let mut bgdt_vec = Vec::with_capacity(num_groups as usize);
        unsafe {
            let src_ptr = bgdt_buf_virt as *const DiskGroupDesc;
            for i in 0..num_groups as usize {
                bgdt_vec.push(ptr::read(src_ptr.add(i)));
            }
        }

        ALLOCATOR.free_order(bgdt_buf_phys, bgdt_alloc_order);

        Ok(Ext2FileSystem {
            partition,
            cache,
            block_size,
            sectors_per_block,
            inodes_per_group: sb.inodes_per_group,
            blocks_per_group: sb.blocks_per_group,
            inode_size,
            bgdt: TicketLock::new(bgdt_vec),
            allocation_lock: AsyncMutex::new(()),
            active_files: Mutex::new(BTreeMap::new()),
            active_dirs: Mutex::new(BTreeMap::new()),
            dirty_files: Mutex::new(BTreeMap::new()),
        })
    }

    fn metadata_block_size(&self) -> Result<u32, Ext2Error> {
        let block_size = self.block_size;
        if !matches!(block_size, 1024 | 2048 | 4096) {
            return Err(Ext2Error::InvalidMetadata);
        }
        Ok(block_size)
    }

    fn inode_location(&self, inode_num: u32) -> Result<(u32, usize), Ext2Error> {
        let block_size = self.metadata_block_size()?;

        if inode_num == 0 || self.inodes_per_group == 0 
            || (self.inode_size as usize) < size_of::<DiskInode>() {
            return Err(Ext2Error::InvalidMetadata);
        };

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

    fn read_dir_header(bytes: &[u8]) -> Result<DiskDirHeader, Ext2Error> {
        let header_size = size_of::<DiskDirHeader>();
        if bytes.len() < header_size {
            return Err(Ext2Error::InvalidMetadata);
        }

        let entry = unsafe { ptr::read(bytes.as_ptr().cast::<DiskDirHeader>()) };
        let rec_len = entry.record_length as usize;
        if rec_len < header_size
            || rec_len % 4 != 0
            || rec_len > bytes.len()
            || entry.name_length as usize > rec_len - header_size
        {
            return Err(Ext2Error::InvalidMetadata);
        }

        Ok(entry)
    }

    pub async fn lookup_in_dir(&self, inode: &DiskInode, name: &str) -> Result<Option<u32>, Ext2Error> {
        const DIRECT_BLOCKS: u32 = 12;
        const HEADER_SIZE: usize = size_of::<DiskDirHeader>();

        let block_size = self.metadata_block_size()?;
        let dir_size = inode.size;
        if dir_size % block_size != 0 {
            return Err(Ext2Error::InvalidMetadata);
        }

        let block_count = dir_size / block_size;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;

        // walk thru 12 direct blk pointers
        for idx in 0..block_count.min(DIRECT_BLOCKS) as usize {
            let block = unsafe { inode.data.blocks.direct[idx] };
            if block == 0 {
                return Err(Ext2Error::InvalidMetadata);
            }

            self.read_metadata_block(block, &buffer).await?;

            let mut bytes = unsafe {
                slice::from_raw_parts(buffer.virt() as *const u8, block_size as usize)
            };

            while !bytes.is_empty() {
                let entry = Self::read_dir_header(bytes)?;
                let rec_len = entry.record_length as usize;
                let name_len = entry.name_length as usize;

                if entry.inode != 0 && &bytes[HEADER_SIZE..HEADER_SIZE + name_len] == name.as_bytes() {
                    return Ok(Some(entry.inode));
                }

                bytes = &bytes[rec_len..];
            }
        }
        
        if block_count > DIRECT_BLOCKS {
            return Err(Ext2Error::UnsupportedDirectoryLayout);
        }

        Ok(None)
    }

    pub async fn resolve_file_block(&self, inode: &DiskInode, file_block_idx: usize) -> Result<u32, Ext2Error> {
        let (depth, indices) = self.indirect_path(file_block_idx)?;

        if depth == 0 {
            return Ok(unsafe { inode.data.blocks.direct[file_block_idx] });
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

    fn add_inode_block(&self, inode: &mut DiskInode) {
        let blocks = inode.blocks;
        inode.blocks = blocks.saturating_add(self.sectors_per_block);
    }

    fn remove_inode_block(&self, inode: &mut DiskInode) {
        let blocks = inode.blocks;
        inode.blocks = blocks.saturating_sub(self.sectors_per_block);
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
            crate::klogln!("[EXT2] could not release unlinked block {}: {:?}", block, error);
        }
    }

    async fn read_pointer(&self, block: u32, index: usize) -> Result<u32, Ext2Error> {
        let pointers = self.metadata_block_size()? as usize / size_of::<u32>();
        if block == 0 || index >= pointers {
            return Err(Ext2Error::InvalidMetadata);
        }

        let page = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        self.read_metadata_block(block, &page).await?;

        Ok(unsafe { ptr::read((page.virt() as *const u32).add(index)) })
    }

    async fn write_pointer(&self, block: u32, index: usize, value: u32) -> Result<bool, Ext2Error> {
        let pointers = self.metadata_block_size()? as usize / size_of::<u32>();
        if block == 0 || index >= pointers {
            return Err(Ext2Error::InvalidMetadata);
        }

        let page = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        self.read_metadata_block(block, &page).await?;

        let empty = unsafe {
            let table = page.virt() as *mut u32;
            ptr::write(table.add(index), value);
            (0..pointers).all(|index| ptr::read(table.add(index)) == 0)
        };

        self.write_metadata_block(block, &page).await?;
        Ok(empty)
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
        let (depth, indices) = self.indirect_path(file_block_idx)?;

        if depth == 0 {
            let existing = unsafe { inode.data.blocks.direct[file_block_idx] };
            if existing != 0 {
                return Ok(existing);
            }

            let data = self.allocate_block().await?;
            unsafe {
                inode.data.blocks.direct[file_block_idx] = data;
            }
            self.add_inode_block(inode);
            return Ok(data);
        }

        let mut table = Self::get_indirect_root(inode, depth);
        let mut parent = None;
        let mut missing_tables = &indices[..depth];
        if table != 0 {
            for (level, &index) in indices[..depth].iter().enumerate() {
                let child = self.read_pointer(table, index).await?;
                if child == 0 {
                    parent = Some((table, index));
                    missing_tables = &indices[level + 1..depth];
                    break;
                }

                if level + 1 == depth {
                    return Ok(child);
                }
                table = child;
            }
        }

        // Stage the data block and any missing tables before publishing their first link.
        let mut staged = [0; 4];
        let mut count = 0;
        let result = async {
            let data = self.allocate_block().await?;
            staged[count] = data;
            count += 1;

            let mut child = data;
            for &index in missing_tables.iter().rev() {
                let table = self.allocate_zeroed_block().await?;
                staged[count] = table;
                count += 1;
                self.write_pointer(table, index, child).await?;
                child = table;
            }

            if let Some((table, index)) = parent {
                self.write_pointer(table, index, child).await?;
            } else {
                Self::set_indirect_root(inode, depth, child);
            }

            for _ in 0..count {
                self.add_inode_block(inode);
            }
            Ok(data)
        }.await;

        if result.is_err() {
            for &block in staged[..count].iter().rev() {
                self.release_unlinked_block(block).await;
            }
        }
        result
    }

    pub async fn clear_file_block(&self, inode: &mut DiskInode, file_block_idx: usize) -> Result<(), Ext2Error> {
        let (depth, indices) = self.indirect_path(file_block_idx)?;
        if depth == 0 {
            let data = unsafe { inode.data.blocks.direct[file_block_idx] };
            if data != 0 {
                unsafe {
                    inode.data.blocks.direct[file_block_idx] = 0;
                }
                self.free_block(data).await?;
                self.remove_inode_block(inode);
            }
            return Ok(());
        }
        let root = Self::get_indirect_root(inode, depth);
        if root == 0 {
            return Ok(());
        }
        let mut tables = [0u32; 3];
        tables[0] = root;
        for level in 0..depth - 1 {
            let child = self.read_pointer(tables[level], indices[level]).await?;
            if child == 0 {
                return Ok(());
            }
            tables[level + 1] = child;
        }
        let leaf_level = depth - 1;
        let data = self.read_pointer(tables[leaf_level], indices[leaf_level]).await?;
        if data == 0 {
            return Ok(());
        }
        let mut empty = self.write_pointer(tables[leaf_level], indices[leaf_level], 0).await?;
        self.free_block(data).await?;
        self.remove_inode_block(inode);
        let mut level = leaf_level;
        while empty {
            self.free_block(tables[level]).await?;
            self.remove_inode_block(inode);
            if level == 0 {
                Self::set_indirect_root(inode, depth, 0);
                break;
            }
            level -= 1;
            empty = self.write_pointer(tables[level], indices[level], 0).await?;
        }
        Ok(())
    }

    pub async fn write_inode(&self, inode_num: u32, inode: &DiskInode) -> Result<(), Ext2Error> {
        let (block, offset) = self.inode_location(inode_num)?;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;

        self.read_metadata_block(block, &buffer).await?;

        unsafe {
            let dest = (buffer.virt() as *mut u8).add(offset).cast::<DiskInode>();
            ptr::write(dest, *inode);
        }

        self.write_metadata_block(block, &buffer).await
    }

    async fn read_metadata_block(&self, block: u32, buffer: &PhysBuffer) -> Result<(), Ext2Error> {
        self.cache.read_block(block as usize, buffer.phys() as u64).await
            .map_err(|_| Ext2Error::BlockRead)
    }

    async fn write_metadata_block(&self, block: u32, buffer: &PhysBuffer) -> Result<(), Ext2Error> {
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

    async fn write_allocation_counts(
        &self,
        group: usize,
        descriptor: DiskGroupDesc,
        buffer: &PhysBuffer,
        update_superblock: impl FnOnce(&mut DiskSuperblock) + Send,
    ) -> Result<(), Ext2Error> {
        let block_size = self.metadata_block_size()? as usize;
        let bgdt_start = if block_size == 1024 { 2 } else { 1 };
        let descriptor_offset = group * size_of::<DiskGroupDesc>();
        let block = bgdt_start + (descriptor_offset / block_size) as u32;
        let offset = descriptor_offset % block_size;

        self.read_metadata_block(block, buffer).await?;
        unsafe {
            let dest = (buffer.virt() as *mut u8).add(offset).cast::<DiskGroupDesc>();
            ptr::write(dest, descriptor);
        }
        self.write_metadata_block(block, buffer).await?;

        let sb_block = (1024 / block_size) as u32;
        let sb_offset = 1024 % block_size;
        self.read_metadata_block(sb_block, buffer).await?;
        unsafe {
            let sb = (buffer.virt() as *mut u8).add(sb_offset).cast::<DiskSuperblock>();
            update_superblock(&mut *sb);
        }
        self.write_metadata_block(sb_block, buffer).await
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
            let index = self.claim_group_bit(bitmap, self.blocks_per_group, &buffer).await?;

            let first_data_block = if self.block_size == 1024 { 1 } else { 0 };
            let block = (group as u32)
                .checked_mul(self.blocks_per_group)
                .and_then(|base| base.checked_add(index))
                .and_then(|block| block.checked_add(first_data_block))
                .filter(|&block| block != 0)
                .ok_or(Ext2Error::InvalidMetadata)?;

            self.write_metadata_block(bitmap, &buffer).await?;

            descriptor.free_blocks_count -= 1;
            self.bgdt.lock()[group] = descriptor;
            self.write_allocation_counts(group, descriptor, &buffer, |sb| {
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

            if is_dir {
                descriptor.used_dirs_count = descriptor.used_dirs_count
                    .checked_add(1)
                    .ok_or(Ext2Error::InvalidMetadata)?;
            }

            self.write_metadata_block(bitmap, &buffer).await?;

            descriptor.free_inodes_count -= 1;
            self.bgdt.lock()[group] = descriptor;
            self.write_allocation_counts(group, descriptor, &buffer, |sb| {
                sb.free_inodes_count = sb.free_inodes_count.saturating_sub(1);
            }).await?;

            return Ok(inode);
        }

        Err(Ext2Error::NoSpace)
    }

    async fn release_group_bit(
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

        self.write_metadata_block(bitmap_block, buffer).await
    }

    pub async fn free_inode(&self, inode_num: u32, is_dir: bool) -> Result<(), Ext2Error> {
        if inode_num == 0 || self.inodes_per_group == 0 {
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
        self.release_group_bit(descriptor.inode_bitmap, index, &buffer).await?;
        self.bgdt.lock()[group] = descriptor;

        self.write_allocation_counts(group, descriptor, &buffer, |sb| {
            sb.free_inodes_count = sb.free_inodes_count.saturating_add(1);
        }).await
    }

    pub async fn free_block(&self, block_id: u32) -> Result<(), Ext2Error> {
        if block_id == 0 {
            return Ok(());
        }
        if self.blocks_per_group == 0 {
            return Err(Ext2Error::InvalidMetadata);
        }

        let _guard = self.allocation_lock.lock().await;
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let first_data_block = u32::from(self.block_size == 1024);
        let relative_block = block_id - first_data_block;
        let group = (relative_block / self.blocks_per_group) as usize;
        let index = relative_block % self.blocks_per_group;

        let mut descriptor = *self.bgdt.lock().get(group).ok_or(Ext2Error::InvalidMetadata)?;
        descriptor.free_blocks_count = descriptor.free_blocks_count
            .checked_add(1)
            .ok_or(Ext2Error::InvalidMetadata)?;
        self.release_group_bit(descriptor.block_bitmap, index, &buffer).await?;
        self.bgdt.lock()[group] = descriptor;

        self.write_allocation_counts(group, descriptor, &buffer, |sb| {
            sb.free_blocks_count = sb.free_blocks_count.saturating_add(1);
        }).await
    }
}
