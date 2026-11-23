use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Debug;
use core::ptr::copy_nonoverlapping;

use crate::executor::async_mutex::AsyncMutex;
use crate::drivers::virtio::blk::BlockTransferFuture;
use crate::memory::{
    ALLOCATOR,
    DIRECT_MAP_OFFSET,
    calculate_order,
};

#[derive(Debug)]
pub struct DmaBuffer {
    phys: usize,
    len: usize,
    order: usize,
}

impl DmaBuffer {
    pub fn new(len: usize) -> Result<Arc<Self>, ()> {
        let alloc_len = len.max(1);
        let order = calculate_order(alloc_len);
        let phys = ALLOCATOR.alloc_order(order).ok_or(())?;
        Ok(Arc::new(Self { phys, len, order }))
    }

    pub fn from_phys(src_phys: usize, len: usize) -> Result<Arc<Self>, ()> {
        let buffer = Self::new(len)?;
        unsafe {
            copy_nonoverlapping((src_phys + *DIRECT_MAP_OFFSET) as *const u8, (buffer.phys + *DIRECT_MAP_OFFSET) as *mut u8, len);
        }
        Ok(buffer)
    }

    pub fn from_slice(src: &[u8]) -> Result<Arc<Self>, ()> {
        let buffer = Self::new(src.len())?;
        unsafe {
            copy_nonoverlapping(src.as_ptr(), (buffer.phys + *DIRECT_MAP_OFFSET) as *mut u8, src.len());
        }
        Ok(buffer)
    }

    pub fn copy_to_phys(&self, dst_phys: usize) {
        unsafe {
            copy_nonoverlapping((self.phys + *DIRECT_MAP_OFFSET) as *const u8, (dst_phys + *DIRECT_MAP_OFFSET) as *mut u8, self.len);
        }
    }

    pub fn copy_to_slice(&self, dst: &mut [u8]) {
        assert!(dst.len() >= self.len, "dma destination slice too small");
        unsafe {
            copy_nonoverlapping((self.phys + *DIRECT_MAP_OFFSET) as *const u8, dst.as_mut_ptr(), self.len);
        }
    }

    pub fn phys(&self) -> usize { self.phys }

    pub fn len(&self) -> usize { self.len }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) { ALLOCATOR.free_order(self.phys, self.order); }
}

pub trait AsyncBlockDevice: Send + Sync + Debug {
    /// Reads are staged through a driver-owned DMA buffer and copied back
    /// into `buf_phys` only when the returned future resolves successfully.
    fn read_sectors(&self, sector: u64, sectors_count: u32, buf_phys: u64) -> Result<BlockTransferFuture, ()>;

    /// Writes are staged through a driver-owned DMA buffer before submission,
    /// so the device does not retain access to `buf_phys` after this call returns.
    fn write_sectors(&self, sector: u64, sectors_count: u32, buf_phys: u64) -> Result<BlockTransferFuture, ()>;

    fn flush(&self) -> Result<BlockTransferFuture, BlockFlushError> {
        Err(BlockFlushError::Unsupported)
    }

    fn sector_size(&self) -> usize { 512 }
}

#[derive(Debug)]
struct CacheEntry {
    block_id: Option<usize>,
    referenced: bool,
    dirty: bool,
    pending_write: Option<BlockTransferFuture>,
    data: Vec<u8>,
}

#[derive(Debug)]
pub struct BlockCache {
    device: Arc<dyn AsyncBlockDevice>,
    block_size: usize,
    sectors_per_block: usize,
    inner: AsyncMutex<BlockCacheInner>,
}

#[derive(Debug)]
struct BlockCacheInner {
    entries: Vec<CacheEntry>,
    clock_hand: usize,
}

#[derive(Debug)]
pub enum BlockFlushError {
    Unsupported,
    Io,
}

impl BlockCacheInner {
    fn find(&self, block_id: usize) -> Option<usize> {
        self.entries.iter().position(|entry| entry.block_id == Some(block_id))
    }

    fn victim(&mut self, reserved: &[usize]) -> usize {
        if let Some(index) = self.entries.iter().position(|entry| entry.block_id.is_none()) {
            return index;
        }

        loop {
            let index = self.clock_hand;
            self.clock_hand = (index + 1) % self.entries.len();
            if !reserved.contains(&index) && !core::mem::replace(&mut self.entries[index].referenced, false) {
                return index;
            }
        }
    }
}

impl BlockCache {
    pub fn new(device: Arc<dyn AsyncBlockDevice>, block_size: usize, num_entries: usize) -> Self {
        assert!(num_entries > 0);
        let sectors_per_block = block_size / device.sector_size();
        let entries = (0..num_entries).map(|_| CacheEntry {
            block_id: None,
            referenced: false,
            dirty: false,
            pending_write: None,
            data: vec![0; block_size],
        }).collect();

        Self {
            device,
            block_size,
            sectors_per_block,
            inner: AsyncMutex::new(BlockCacheInner { entries, clock_hand: 0 }),
        }
    }

    async fn finish_write(entry: &mut CacheEntry) -> Result<(), ()> {
        if let Some(transfer) = entry.pending_write.as_mut() {
            let result = transfer.await;
            entry.pending_write = None;
            result?;
            entry.dirty = false;
        }

        Ok(())
    }

    async fn flush_entry(&self, entry: &mut CacheEntry) -> Result<(), ()> {
        if !entry.dirty {
            return Ok(());
        }

        if entry.pending_write.is_none() {
            let block_id = entry.block_id.expect("dirty cache entry without a block");
            let sector = block_id as u64 * self.sectors_per_block as u64;
            let dma = DmaBuffer::from_slice(&entry.data)?;
            entry.pending_write = Some(self.device.write_sectors(
                sector, self.sectors_per_block as u32, dma.phys() as u64,
            )?);
        }

        Self::finish_write(entry).await
    }

    async fn load_entry(
        &self,
        inner: &mut BlockCacheInner,
        block_id: usize,
        reserved: &[usize],) -> Result<usize, ()> {
        if let Some(index) = inner.find(block_id) {
            return Ok(index);
        }

        let index = inner.victim(reserved);
        let entry = &mut inner.entries[index];
        self.flush_entry(entry).await?;

        let dma = DmaBuffer::new(self.block_size)?;
        let sector = block_id as u64 * self.sectors_per_block as u64;
        self.device.read_sectors(sector, self.sectors_per_block as u32, dma.phys() as u64)?.await?;

        dma.copy_to_slice(&mut entry.data);
        entry.block_id = Some(block_id);
        Ok(index)
    }

    /// Load and reserve every block before editing any of them.
    /// The edit callback runs under the cache lock and must be infallible.
    pub(crate) async fn update_blocks(
        &self,
        blocks: &[usize],
        mut edit: impl FnMut(usize, &mut [u8]) + Send,
    ) -> Result<(), ()> {
        let mut inner = self.inner.lock().await;
        let mut reserved = [0; 4];
        if blocks.len() > inner.entries.len() || blocks.len() > reserved.len() {
            return Err(());
        }
        for (position, &block) in blocks.iter().enumerate() {
            if blocks[..position].contains(&block) {
                return Err(());
            }
            let slot = self.load_entry(&mut inner, block, &reserved[..position]).await?;
            Self::finish_write(&mut inner.entries[slot]).await?;
            reserved[position] = slot;
        }

        for (&block, slot) in blocks.iter().zip(reserved) {
            let entry = &mut inner.entries[slot];
            edit(block, &mut entry.data);
            entry.referenced = true;
            entry.dirty = true;
        }
        Ok(())
    }

    pub async fn read_block(&self, block_id: usize, dest_phys: u64) -> Result<(), ()> {
        let mut inner = self.inner.lock().await;
        let index = self.load_entry(&mut inner, block_id, &[]).await?;
        let entry = &mut inner.entries[index];
        entry.referenced = true;
        let dest_virt = dest_phys + *DIRECT_MAP_OFFSET as u64;
        unsafe {
            copy_nonoverlapping(entry.data.as_ptr(), dest_virt as *mut u8, self.block_size);
        }
        Ok(())
    }

    pub async fn write_block(&self, block_id: usize, src_phys: u64) -> Result<(), ()> {
        let mut inner = self.inner.lock().await;
        let index = match inner.find(block_id) {
            Some(index) => {
                Self::finish_write(&mut inner.entries[index]).await?;
                index
            }
            None => {
                let index = inner.victim(&[]);
                self.flush_entry(&mut inner.entries[index]).await?;
                index
            }
        };

        let entry = &mut inner.entries[index];
        let src_virt = src_phys + *DIRECT_MAP_OFFSET as u64;
        unsafe {
            copy_nonoverlapping(src_virt as *const u8, entry.data.as_mut_ptr(), self.block_size);
        }
        entry.block_id = Some(block_id);
        entry.referenced = true;
        entry.dirty = true;
        Ok(())
    }

    pub async fn flush(&self) -> Result<(), ()> {
        let mut inner = self.inner.lock().await;
        for entry in &mut inner.entries {
            self.flush_entry(entry).await?;
        }
        Ok(())
    }

    pub async fn invalidate(&self, block_id: usize) {
        let mut inner = self.inner.lock().await;
        if let Some(index) = inner.find(block_id) {
            let entry = &mut inner.entries[index];

            // wait for a submitted write to finish before allowing block reuse.
            // its status is irrelevant because the old contents are being discarded.
            let _ = Self::finish_write(entry).await;
            entry.block_id = None;
            entry.dirty = false;
            entry.referenced = false;
        }
    }
}
