use alloc::boxed::Box;
use alloc::vec::Vec;
use core::{ptr, slice};

use crate::memory::PhysBuffer;
use crate::time::get_realtime;

use super::structs::DiskInode;
use super::{Ext2Error, Ext2FileSystem};

#[derive(Debug)]
pub(super) enum ReclaimJob {
    Orphan { inode_num: u32, inode: DiskInode },
    Blocks { blocks: Vec<u32>, release_inode: Option<(u32, bool)> },
}

pub(super) struct TruncatePlan {
    pub inode: DiskInode,
    pub edits: Vec<PointerEdit>,
    pub retired: Vec<u32>,
}

pub(super) enum PointerEdit {
    ClearSuffix { block: u32, first: usize },
    Set { block: u32, index: usize, value: u32 },
}

impl PointerEdit {
    fn block(&self) -> u32 {
        match *self {
            Self::ClearSuffix { block, .. } | Self::Set { block, .. } => block,
        }
    }

    pub(super) fn apply(&self, bytes: &mut [u8]) {
        match *self {
            Self::ClearSuffix { first, .. } => bytes[first * size_of::<u32>()..].fill(0),
            Self::Set { index, value, .. } => {
                let offset = index * size_of::<u32>();
                bytes[offset..offset + size_of::<u32>()].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}

fn push<T>(vec: &mut Vec<T>, item: T) -> Result<(), Ext2Error> {
    vec.try_reserve(1).map_err(|_| Ext2Error::OutOfMemory)?;
    vec.push(item);
    Ok(())
}

impl Ext2FileSystem {
    pub(super) async fn plan_shrink(&self, mut inode: DiskInode, size: u32) -> Result<TruncatePlan, Ext2Error> {
        let block_size = self.metadata_block_size()? as usize;
        let fanout = block_size / size_of::<u32>();
        let keep = (size as usize).div_ceil(block_size);
        let mut edits = Vec::new();
        let mut retired = Vec::new();

        for index in keep.min(12)..12 {
            let block = unsafe { inode.data.blocks.direct[index] };
            if block != 0 {
                push(&mut retired, block)?;
                unsafe { inode.data.blocks.direct[index] = 0 };
            }
        }

        let mut start = 12;
        for depth in 1..=3 {
            let root = Self::get_indirect_root(&inode, depth);
            let span = fanout.pow(depth as u32);
            let keep_in_tree = keep.saturating_sub(start).min(span);
            start += span;
            if root == 0 || keep_in_tree == span {
                continue;
            }

            if keep_in_tree == 0 {
                self.collect_subtree(root, depth, fanout, &mut retired).await?;
                Self::set_indirect_root(&mut inode, depth, 0);
            } else if !self.plan_table_trim(root, depth, fanout, keep_in_tree, &mut edits, &mut retired).await? {
                Self::set_indirect_root(&mut inode, depth, 0);
            }
        }

        // A repeated retired pointer would let a retry free somebody else's block.
        retired.sort_unstable();
        if retired.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Ext2Error::InvalidMetadata);
        }
        let sectors = u32::try_from(retired.len()).ok()
            .and_then(|count| count.checked_mul(self.sectors_per_block))
            .ok_or(Ext2Error::InvalidMetadata)?;
        inode.blocks = inode.blocks.checked_sub(sectors).ok_or(Ext2Error::InvalidMetadata)?;
        inode.size = size;

        Ok(TruncatePlan { inode, edits, retired })
    }

    async fn plan_table_trim(
        &self,
        block: u32,
        depth: usize,
        fanout: usize,
        keep: usize,
        edits: &mut Vec<PointerEdit>,
        retired: &mut Vec<u32>,
    ) -> Result<bool, Ext2Error> {
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        self.read_metadata_block(block, &buffer).await?;
        let pointers = unsafe { slice::from_raw_parts(buffer.virt() as *const u32, fanout) };

        let child_span = fanout.pow((depth - 1) as u32);
        let whole = keep / child_span;
        let remainder = keep % child_span;
        let mut any_kept = pointers[..whole].iter().any(|&pointer| pointer != 0);
        let mut clear_from = whole;

        // Only one child can straddle the new EOF.
        if remainder != 0 {
            let child = pointers[whole];
            if child != 0
                && Box::pin(self.plan_table_trim(child, depth - 1, fanout, remainder, edits, retired)).await?
            {
                any_kept = true;
                clear_from += 1;
            }
        }
        for &child in &pointers[keep.div_ceil(child_span)..] {
            if child != 0 {
                self.collect_subtree(child, depth - 1, fanout, retired).await?;
            }
        }

        if any_kept {
            if pointers[clear_from..].iter().any(|&pointer| pointer != 0) {
                push(edits, PointerEdit::ClearSuffix { block, first: clear_from })?;
            }
        } else {
            push(retired, block)?;
        }

        Ok(any_kept)
    }

    async fn collect_subtree(
        &self,
        root: u32,
        depth: usize,
        fanout: usize,
        retired: &mut Vec<u32>,
    ) -> Result<(), Ext2Error> {
        if depth == 0 {
            return push(retired, root);
        }

        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let mut pending = Vec::new();
        push(&mut pending, (root, depth))?;
        while let Some((block, depth)) = pending.pop() {
            push(retired, block)?;
            self.read_metadata_block(block, &buffer).await?;
            let pointers = unsafe { slice::from_raw_parts(buffer.virt() as *const u32, fanout) };
            for &child in pointers.iter().filter(|&&pointer| pointer != 0) {
                if depth == 1 {
                    push(retired, child)?;
                } else {
                    push(&mut pending, (child, depth - 1))?;
                }
            }
        }
        Ok(())
    }

    pub(super) async fn write_inode_and_tables(
        &self,
        inode_num: u32,
        inode: &DiskInode,
        edits: &[PointerEdit],
    ) -> Result<(), Ext2Error> {
        let mut blocks = [0; 3];
        if edits.len() > blocks.len() {
            return Err(Ext2Error::InvalidMetadata);
        }
        for (key, edit) in blocks.iter_mut().zip(edits) {
            *key = edit.block();
        }
        self.write_inode_with_blocks(inode_num, inode, &blocks[..edits.len()], |id, bytes| {
            let edit = edits.iter().find(|edit| edit.block() == id).unwrap();
            edit.apply(bytes);
        }).await
    }

    pub(super) async fn write_inode_with_blocks(
        &self,
        inode_num: u32,
        inode: &DiskInode,
        extra_blocks: &[u32],
        mut edit: impl FnMut(u32, &mut [u8]) + Send,
    ) -> Result<(), Ext2Error> {
        let (block, offset) = self.inode_location(inode_num)?;
        let block = block as usize;
        let mut keys = [0; 4];
        if extra_blocks.len() >= keys.len() || extra_blocks.iter().any(|&id| id >= self.blocks_count) {
            return Err(Ext2Error::InvalidMetadata);
        }
        keys[0] = block;
        for (key, &id) in keys[1..].iter_mut().zip(extra_blocks) {
            *key = id as usize;
        }

        self.cache.update_blocks(&keys[..=extra_blocks.len()], |id, bytes| {
            if id == block {
                unsafe { ptr::write(bytes.as_mut_ptr().add(offset).cast::<DiskInode>(), *inode) };
            } else {
                edit(id as u32, bytes);
            }
        }).await.map_err(|_| Ext2Error::BlockWrite)
    }

    pub(super) fn queue_orphan(&self, inode_num: u32, inode: DiskInode) {
        self.reclaim_queue.lock().push_back(ReclaimJob::Orphan { inode_num, inode });
    }

    pub(super) fn queue_retired_blocks(&self, blocks: Vec<u32>) {
        if !blocks.is_empty() {
            self.reclaim_queue.lock().push_back(ReclaimJob::Blocks { blocks, release_inode: None });
        }
    }

    pub(crate) async fn reap_pending(&self) {
        let mut jobs = core::mem::take(&mut *self.reclaim_queue.lock());
        while let Some(mut job) = jobs.pop_front() {
            if let Err(error) = self.reap(&mut job).await {
                crate::klogln!("[EXT2] reclamation deferred: {:?}", error);
                self.reclaim_queue.lock().push_back(job);
            }
        }
    }

    async fn reap(&self, job: &mut ReclaimJob) -> Result<(), Ext2Error> {
        if let ReclaimJob::Orphan { inode_num, inode } = job {
            let mut plan = self.plan_shrink(*inode, 0).await?;
            plan.inode.dtime = get_realtime().0 as u32;
            self.write_inode_and_tables(*inode_num, &plan.inode, &plan.edits).await?;
            let is_dir = inode.mode & 0xf000 == 0x4000;
            *job = ReclaimJob::Blocks { blocks: plan.retired, release_inode: Some((*inode_num, is_dir)) };
        }

        let ReclaimJob::Blocks { blocks, release_inode } = job else { unreachable!() };
        // Old references must reach the device before any bitmap bit is cleared.
        self.cache.flush().await.map_err(|_| Ext2Error::BlockWrite)?;

        while let Some(&block) = blocks.last() {
            self.free_block(block).await?;
            blocks.pop();
        }
        if let Some((inode_num, is_dir)) = *release_inode {
            self.free_inode(inode_num, is_dir).await?;
            *release_inode = None;
        }
        Ok(())
    }
}
