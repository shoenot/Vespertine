use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Debug;
use core::future::Future;
use core::pin::Pin;
use core::ptr::copy_nonoverlapping;
use core::task::{
    Context,
    Poll,
};

use crate::core::sync::TicketLock;
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

    fn sector_size(&self) -> usize { 512 }
}

pub struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

pub fn yield_now() -> YieldNow { YieldNow { yielded: false } }

#[derive(Debug)]
pub struct CacheEntry {
    block_id: Option<usize>,
    referenced: bool,
    dirty: bool,
    in_flight: bool,
    pub version: usize,
    data: Vec<u8>,
}

#[derive(Debug)]
pub struct BlockCache {
    device: Arc<dyn AsyncBlockDevice>,
    block_size: usize,
    sectors_per_block: usize,
    inner: TicketLock<BlockCacheInner>,
}

#[derive(Debug)]
pub struct BlockCacheInner {
    entries: Vec<CacheEntry>,
    clock_hand: usize,
}

impl BlockCache {
    pub fn new(device: Arc<dyn AsyncBlockDevice>, block_size: usize, num_entries: usize) -> Self {
        let sectors_per_block = block_size / 512;
        let mut entries = Vec::with_capacity(num_entries);
        for _ in 0..num_entries {
            entries.push(CacheEntry {
                block_id: None,
                referenced: false,
                dirty: false,
                in_flight: false,
                version: 0,
                data: vec![0; block_size],
            });
        }

        Self { device, block_size, sectors_per_block, inner: TicketLock::new(BlockCacheInner { entries, clock_hand: 0 }) }
    }

    pub async fn read_block(&self, block_id: usize, dest_phys: u64) -> Result<(), ()> {
        loop {
            let is_in_flight = {
                let mut inner = self.inner.lock();
                if let Some(i) = inner.entries.iter().position(|e| e.block_id == Some(block_id)) {
                    if inner.entries[i].in_flight {
                        true
                    } else {
                        // cache hit
                        inner.entries[i].referenced = true;
                        unsafe {
                            let dest_virt = dest_phys + *DIRECT_MAP_OFFSET as u64;
                            copy_nonoverlapping(inner.entries[i].data.as_ptr(), dest_virt as *mut u8, self.block_size);
                        }
                        return Ok(());
                    }
                } else {
                    break; // cache miss, evict
                }
            };

            if is_in_flight {
                yield_now().await;
            }
        }

        // cache miss
        let mut selected_idx = None;
        let mut old_writeback = None;

        {
            let mut inner = self.inner.lock();
            for (i, entry) in inner.entries.iter().enumerate() {
                if entry.block_id.is_none() && !entry.in_flight {
                    selected_idx = Some(i);
                    break;
                }
            }

            if selected_idx.is_none() {
                let num_entries = inner.entries.len();
                let mut checked = 0;
                loop {
                    let i = inner.clock_hand;

                    if inner.entries[i].in_flight {
                        inner.clock_hand = (inner.clock_hand + 1) % num_entries;
                        checked += 1;
                        if checked >= num_entries {
                            return Err(());
                        }
                        continue;
                    }

                    if !inner.entries[i].referenced {
                        selected_idx = Some(i);
                        inner.clock_hand = (inner.clock_hand + 1) % num_entries;
                        break;
                    } else {
                        inner.entries[i].referenced = false;
                        inner.clock_hand = (inner.clock_hand + 1) % num_entries;
                    }
                }
            }

            if let Some(idx) = selected_idx {
                inner.entries[idx].in_flight = true;
                let entry = &inner.entries[idx];
                if entry.dirty {
                    if let Some(old_block) = entry.block_id {
                        old_writeback = Some((old_block, entry.data.clone()));
                    }
                }
            }
        }

        let idx = selected_idx.ok_or(())?;

        // eviction writeback
        if let Some((old_block, old_data)) = old_writeback {
            let start_sector = old_block as u64 * self.sectors_per_block as u64;
            let dma = DmaBuffer::from_slice(old_data.as_slice())?;
            let write_future = self.device.write_sectors(start_sector, self.sectors_per_block as u32, dma.phys() as u64);
            if write_future.is_err() || write_future.unwrap().await.is_err() {
                let mut inner = self.inner.lock();
                inner.entries[idx].in_flight = false;
                return Err(());
            }
            drop(old_data);

            {
                let mut inner = self.inner.lock();
                inner.entries[idx].dirty = false;
            }
        }

        {
            let mut inner = self.inner.lock();
            inner.entries[idx].block_id = Some(block_id);
        }

        let new_sector = block_id as u64 * self.sectors_per_block as u64;
        let dma = DmaBuffer::new(self.block_size)?;
        let read_future = self.device.read_sectors(new_sector, self.sectors_per_block as u32, dma.phys() as u64);
        if read_future.is_err() || read_future.unwrap().await.is_err() {
            let mut inner = self.inner.lock();
            inner.entries[idx].in_flight = false;
            return Err(());
        }

        {
            let mut inner = self.inner.lock();
            let entry = &mut inner.entries[idx];
            dma.copy_to_slice(entry.data.as_mut_slice());
            entry.referenced = true;
            entry.in_flight = false;
            entry.version += 1; // mark slot as changed

            unsafe {
                let dest_virt = dest_phys + *DIRECT_MAP_OFFSET as u64;
                copy_nonoverlapping(entry.data.as_ptr(), dest_virt as *mut u8, self.block_size);
            }
        }

        Ok(())
    }

    pub async fn write_block(&self, block_id: usize, src_phys: u64) -> Result<(), ()> {
        loop {
            let is_in_flight = {
                let mut inner = self.inner.lock();
                if let Some(i) = inner.entries.iter().position(|e| e.block_id == Some(block_id)) {
                    if inner.entries[i].in_flight {
                        true
                    } else {
                        // cache hit
                        inner.entries[i].referenced = true;
                        inner.entries[i].dirty = true;
                        inner.entries[i].version += 1;

                        unsafe {
                            let src_virt = src_phys + *DIRECT_MAP_OFFSET as u64;
                            copy_nonoverlapping(src_virt as *const u8, inner.entries[i].data.as_mut_ptr(), self.block_size);
                        }
                        return Ok(());
                    }
                } else {
                    break;
                }
            };

            if is_in_flight {
                yield_now().await;
            }
        }

        // cache miss
        let mut selected_idx = None;
        let mut old_writeback = None;

        {
            let mut inner = self.inner.lock();
            for (i, entry) in inner.entries.iter().enumerate() {
                if entry.block_id.is_none() && !entry.in_flight {
                    selected_idx = Some(i);
                    break;
                }
            }

            if selected_idx.is_none() {
                let num_entries = inner.entries.len();
                let mut checked = 0;
                loop {
                    let i = inner.clock_hand;

                    if inner.entries[i].in_flight {
                        inner.clock_hand = (inner.clock_hand + 1) % num_entries;
                        checked += 1;
                        if checked >= num_entries {
                            return Err(());
                        }
                        continue;
                    }

                    if !inner.entries[i].referenced {
                        selected_idx = Some(i);
                        inner.clock_hand = (inner.clock_hand + 1) % num_entries;
                        break;
                    } else {
                        inner.entries[i].referenced = false;
                        inner.clock_hand = (inner.clock_hand + 1) % num_entries;
                    }
                }
            }

            if let Some(idx) = selected_idx {
                inner.entries[idx].in_flight = true;
                let entry = &inner.entries[idx];
                if entry.dirty {
                    if let Some(old_block) = entry.block_id {
                        old_writeback = Some((old_block, entry.data.clone()));
                    }
                }
            }
        }

        let idx = selected_idx.ok_or(())?;

        if let Some((old_block, old_data)) = old_writeback {
            let start_sector = old_block as u64 * self.sectors_per_block as u64;
            let dma = DmaBuffer::from_slice(old_data.as_slice())?;
            let write_future = self.device.write_sectors(start_sector, self.sectors_per_block as u32, dma.phys() as u64);
            if write_future.is_err() || write_future.unwrap().await.is_err() {
                let mut inner = self.inner.lock();
                inner.entries[idx].in_flight = false;
                return Err(());
            }

            {
                let mut inner = self.inner.lock();
                inner.entries[idx].dirty = false;
            }
        }

        {
            let mut inner = self.inner.lock();
            let entry = &mut inner.entries[idx];
            entry.block_id = Some(block_id);
            entry.referenced = true;
            entry.dirty = true;
            entry.in_flight = false;
            entry.version += 1;

            unsafe {
                let src_virt = src_phys + *DIRECT_MAP_OFFSET as u64;
                copy_nonoverlapping(src_virt as *const u8, entry.data.as_mut_ptr(), self.block_size);
            }
        }

        Ok(())
    }

    pub async fn flush(&self) -> Result<(), ()> {
        let mut dirty_entries = Vec::new();
        {
            let inner = self.inner.lock();
            for (idx, entry) in inner.entries.iter().enumerate() {
                if entry.dirty && !entry.in_flight {
                    if let Some(block_id) = entry.block_id {
                        dirty_entries.push((idx, block_id, entry.version, entry.data.clone()));
                    }
                }
            }
        }

        for (idx, block_id, version, data) in dirty_entries {
            let start_sector = block_id as u64 * self.sectors_per_block as u64;
            let dma = DmaBuffer::from_slice(data.as_slice())?;
            let write_future = self.device.write_sectors(start_sector, self.sectors_per_block as u32, dma.phys() as u64)?;
            write_future.await?;

            // reset the dirty flag only if the version matches the version flushed to disk
            {
                let mut inner = self.inner.lock();
                let entry = &mut inner.entries[idx];
                if entry.block_id == Some(block_id) && entry.version == version {
                    entry.dirty = false;
                }
            }
        }

        Ok(())
    }
}
