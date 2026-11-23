use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr;

use crate::executor::async_mutex::AsyncMutex;
use crate::memory::PhysBuffer;
use crate::storage::blockdev::{AsyncBlockDevice, BlockCache};
use crate::sync::{Mutex, TicketLock};

use super::Ext2FileSystem;
use super::structs::{DiskGroupDesc, DiskInode, DiskSuperblock};

const SECTOR_SIZE: u32 = 512;
const FIRST_DYNAMIC_INODE: u32 = 11;
const COMPAT_HAS_JOURNAL: u32 = 0x0004;
const COMPAT_RESIZE_INODE: u32 = 0x0010;
const COMPAT_SPARSE_SUPER2: u32 = 0x0200;
const INCOMPAT_FILETYPE: u32 = 0x0002;
const RO_COMPAT_SPARSE_SUPER: u32 = 0x0001;
const RO_COMPAT_LARGE_FILE: u32 = 0x0002;

#[derive(Debug)]
pub(super) enum Ext2MountError {
    InvalidSuperblock(&'static str),
    UnsupportedFeatures { compat: u32, incompat: u32, ro_compat: u32 },
    InvalidGroupDescriptor(u32),
    OutOfMemory,
    BlockRead,
}

struct MountGeometry {
    block_size: u32,
    blocks_count: u32,
    blocks_per_group: u32,
    first_data_block: u32,
    inodes_count: u32,
    inodes_per_group: u32,
    inode_size: u32,
    first_inode: u32,
    groups: u32,
    descriptor_blocks: u32,
    inode_table_blocks: u32,
    reserved_gdt_blocks: u32,
    sparse_super: bool,
    directory_file_types: bool,
}

impl MountGeometry {
    fn validate(sb: &DiskSuperblock, partition_sectors: u64) -> Result<Self, Ext2MountError> {
        use Ext2MountError::InvalidSuperblock;

        if sb.magic != 0xef53 {
            return Err(InvalidSuperblock("bad magic"));
        }
        if sb.rev_level > 1 {
            return Err(InvalidSuperblock("unsupported revision"));
        }
        if sb.log_block_size > 2 {
            return Err(InvalidSuperblock("unsupported block size"));
        }
        let block_size = 1024u32 << sb.log_block_size;
        let first_data_block = u32::from(block_size == 1024);
        if sb.first_data_block != first_data_block {
            return Err(InvalidSuperblock("invalid first data block"));
        }

        let blocks_count = sb.blocks_count;
        if blocks_count <= first_data_block
            || u64::from(blocks_count) * u64::from(block_size / SECTOR_SIZE) > partition_sectors
        {
            return Err(InvalidSuperblock("block count outside the partition"));
        }
        let blocks_per_group = sb.blocks_per_group;
        if blocks_per_group == 0 || blocks_per_group > block_size * 8 {
            return Err(InvalidSuperblock("invalid blocks per group"));
        }
        if sb.log_frag_size != sb.log_block_size || sb.frags_per_group != blocks_per_group {
            return Err(InvalidSuperblock("fragments are unsupported"));
        }

        let (inode_size, first_inode, compat, incompat, ro_compat) = if sb.rev_level == 0 {
            (size_of::<DiskInode>() as u32, FIRST_DYNAMIC_INODE, 0, 0, 0)
        } else {
            (u32::from(sb.inode_size), sb.first_ino, sb.feature_compat, sb.feature_incompat, sb.feature_ro_compat)
        };
        let unsupported_compat = compat & (COMPAT_HAS_JOURNAL | COMPAT_SPARSE_SUPER2);
        let unsupported_incompat = incompat & !INCOMPAT_FILETYPE;
        let unsupported_ro_compat = ro_compat & !(RO_COMPAT_SPARSE_SUPER | RO_COMPAT_LARGE_FILE);
        if (unsupported_compat | unsupported_incompat | unsupported_ro_compat) != 0 {
            return Err(Ext2MountError::UnsupportedFeatures {
                compat: unsupported_compat,
                incompat: unsupported_incompat,
                ro_compat: unsupported_ro_compat,
            });
        }
        if sb.rev_level != 0 && sb.last_orphan != 0 {
            return Err(InvalidSuperblock("on-disk orphan list requires recovery"));
        }
        if !inode_size.is_power_of_two() || inode_size < size_of::<DiskInode>() as u32 || inode_size > block_size {
            return Err(InvalidSuperblock("invalid inode size"));
        }

        let inodes_per_group = sb.inodes_per_group;
        let inodes_per_block = block_size / inode_size;
        if inodes_per_group == 0 || inodes_per_group > block_size * 8 || inodes_per_group % inodes_per_block != 0 {
            return Err(InvalidSuperblock("invalid inodes per group"));
        }

        let groups = (blocks_count - first_data_block).div_ceil(blocks_per_group);
        let inodes_count = sb.inodes_count;
        if groups.checked_mul(inodes_per_group) != Some(inodes_count)
            || first_inode < FIRST_DYNAMIC_INODE
            || first_inode > inodes_count
        {
            return Err(InvalidSuperblock("invalid inode count or first usable inode"));
        }
        if sb.free_blocks_count > blocks_count - first_data_block
            || sb.r_blocks_count > blocks_count
            || sb.free_inodes_count > inodes_count
        {
            return Err(InvalidSuperblock("invalid allocation counts"));
        }

        let descriptors_per_block = block_size / size_of::<DiskGroupDesc>() as u32;
        let descriptor_blocks = groups.div_ceil(descriptors_per_block);
        let inode_table_blocks = inodes_per_group / inodes_per_block;
        let reserved_gdt_blocks = if compat & COMPAT_RESIZE_INODE != 0 {
            u32::from(sb.reserved_gdt_blocks)
        } else {
            0
        };
        let group_zero_size = blocks_per_group.min(blocks_count - first_data_block);
        if u64::from(descriptor_blocks) + u64::from(inode_table_blocks) + u64::from(reserved_gdt_blocks) + 3
            > u64::from(group_zero_size)
        {
            return Err(InvalidSuperblock("metadata does not fit in the first group"));
        }

        Ok(Self {
            block_size, blocks_count, blocks_per_group, first_data_block,
            inodes_count, inodes_per_group, inode_size, first_inode, groups,
            descriptor_blocks, inode_table_blocks, reserved_gdt_blocks,
            sparse_super: ro_compat & RO_COMPAT_SPARSE_SUPER != 0,
            directory_file_types: incompat & INCOMPAT_FILETYPE != 0,
        })
    }

    fn validate_group(&self, group: u32, descriptor: &DiskGroupDesc) -> Result<(), Ext2MountError> {
        let error = Ext2MountError::InvalidGroupDescriptor(group);
        let start = self.first_data_block + group * self.blocks_per_group;
        let count = self.blocks_per_group.min(self.blocks_count - start);
        let end = start + count;
        let has_superblock = !self.sparse_super || group <= 1
            || [3, 5, 7].into_iter().any(|base| is_power_of(group, base));
        let reserved = if has_superblock { 1 + self.descriptor_blocks + self.reserved_gdt_blocks } else { 0 };
        let first = start.checked_add(reserved).ok_or(Ext2MountError::InvalidGroupDescriptor(group))?;

        let block_bitmap = descriptor.block_bitmap;
        let inode_bitmap = descriptor.inode_bitmap;
        let inode_table = descriptor.inode_table;
        let table_end = inode_table.checked_add(self.inode_table_blocks)
            .ok_or(Ext2MountError::InvalidGroupDescriptor(group))?;
        let table = inode_table..table_end;
        if !(first..end).contains(&block_bitmap)
            || !(first..end).contains(&inode_bitmap)
            || inode_table < first || table_end > end
            || block_bitmap == inode_bitmap
            || table.contains(&block_bitmap) || table.contains(&inode_bitmap)
            || u32::from(descriptor.free_blocks_count) > count
            || u32::from(descriptor.free_inodes_count) > self.inodes_per_group
            || u32::from(descriptor.used_dirs_count) > self.inodes_per_group - u32::from(descriptor.free_inodes_count)
        {
            return Err(error);
        }
        Ok(())
    }
}

fn is_power_of(mut group: u32, base: u32) -> bool {
    while group > 1 && group % base == 0 {
        group /= base;
    }
    group == 1
}

impl Ext2FileSystem {
    pub(super) async fn mount(
        partition: Arc<dyn AsyncBlockDevice>,
        partition_sectors: u64,
    ) -> Result<Self, Ext2MountError> {
        if partition.sector_size() != SECTOR_SIZE as usize || partition_sectors < 4 {
            return Err(Ext2MountError::InvalidSuperblock("unsupported partition geometry"));
        }

        let buffer = PhysBuffer::new().ok_or(Ext2MountError::OutOfMemory)?;
        partition.read_sectors(2, 2, buffer.phys() as u64).map_err(|_| Ext2MountError::BlockRead)?
            .await.map_err(|_| Ext2MountError::BlockRead)?;
        let geometry = {
            let sb = unsafe { &*(buffer.virt() as *const DiskSuperblock) };
            MountGeometry::validate(sb, partition_sectors)?
        };

        let sectors_per_block = geometry.block_size / SECTOR_SIZE;
        let descriptors_per_block = geometry.block_size as usize / size_of::<DiskGroupDesc>();
        let mut descriptors = Vec::new();
        descriptors.try_reserve_exact(geometry.groups as usize).map_err(|_| Ext2MountError::OutOfMemory)?;
        let descriptor_start = geometry.first_data_block + 1;

        // Read only the blocks containing descriptors, reusing the same physical buffer.
        for block in descriptor_start..descriptor_start + geometry.descriptor_blocks {
            let sector = u64::from(block) * u64::from(sectors_per_block);
            partition.read_sectors(sector, sectors_per_block, buffer.phys() as u64)
                .map_err(|_| Ext2MountError::BlockRead)?
                .await.map_err(|_| Ext2MountError::BlockRead)?;

            let count = (geometry.groups as usize - descriptors.len()).min(descriptors_per_block);
            for index in 0..count {
                let descriptor = unsafe { ptr::read((buffer.virt() as *const DiskGroupDesc).add(index)) };
                geometry.validate_group(descriptors.len() as u32, &descriptor)?;
                descriptors.push(descriptor);
            }
        }

        let cache = BlockCache::new(partition.clone(), geometry.block_size as usize, 512);
        Ok(Self {
            partition,
            cache,
            block_size: geometry.block_size,
            sectors_per_block,
            blocks_count: geometry.blocks_count,
            first_data_block: geometry.first_data_block,
            inodes_count: geometry.inodes_count,
            first_inode: geometry.first_inode,
            inodes_per_group: geometry.inodes_per_group,
            blocks_per_group: geometry.blocks_per_group,
            inode_size: geometry.inode_size,
            directory_file_types: geometry.directory_file_types,
            bgdt: TicketLock::new(descriptors),
            allocation_lock: AsyncMutex::new(()),
            namespace_lock: AsyncMutex::new(()),
            reclaim_queue: TicketLock::new(VecDeque::new()),
            active_files: Mutex::new(BTreeMap::new()),
            active_dirs: Mutex::new(BTreeMap::new()),
            dirty_files: Mutex::new(BTreeMap::new()),
        })
    }
}
