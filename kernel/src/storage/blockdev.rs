use core::fmt::Debug;
use crate::drivers::virtio::blk::BlockTransferFuture;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::ptr::copy_nonoverlapping;
use core::task::{
    Context,
    Poll,
};

use crate::core::sync::TicketLock;
use crate::memory::HHDMOFFSET;


pub trait AsyncBlockDevice: Send + Sync + Debug {
    fn read_sectors(&self, sector: u64, sectors_count: u32, buf_phys: u64) -> Result<BlockTransferFuture, ()>;

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
            entries.push(CacheEntry { block_id: None, referenced: false, dirty: false, in_flight: false, version: 0, data: vec![0; block_size] });
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
                            let dest_virt = dest_phys + *HHDMOFFSET as u64;
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
            let buffer_phys = old_data.as_ptr() as usize - *HHDMOFFSET;
            let write_future = self.device.write_sectors(start_sector, self.sectors_per_block as u32, buffer_phys as u64)?;
            write_future.await?;

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
        let entry_phys = {
            let inner = self.inner.lock();
            inner.entries[idx].data.as_ptr() as usize - *HHDMOFFSET
        };

        let read_future = self.device.read_sectors(new_sector, self.sectors_per_block as u32, entry_phys as u64)?;
        read_future.await?;

        {
            let mut inner = self.inner.lock();
            let entry = &mut inner.entries[idx];
            entry.referenced = true;
            entry.in_flight = false;
            entry.version += 1; // mark slot as changed

            unsafe {
                let dest_virt = dest_phys + *HHDMOFFSET as u64;
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
                            let src_virt = src_phys + *HHDMOFFSET as u64;
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
            let buffer_phys = old_data.as_ptr() as usize - *HHDMOFFSET;
            let write_future = self.device.write_sectors(start_sector, self.sectors_per_block as u32, buffer_phys as u64)?;
            write_future.await?;

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
                let src_virt = src_phys + *HHDMOFFSET as u64;
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
            let buffer_phys = data.as_ptr() as usize - *HHDMOFFSET;
            let write_future = self.device.write_sectors(start_sector, self.sectors_per_block as u32, buffer_phys as u64)?;
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
