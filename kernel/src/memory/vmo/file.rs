use alloc::{boxed::Box, collections::BTreeMap, sync::{Arc, Weak}, vec::Vec};

use crate::{executor::async_mutex::AsyncMutex, memory::{Frame4K, PAGE_SIZE_4K, vmo::MappingPage}, sync::TicketLock};

use crate::{executor::syscall_bridge::block_on, memory::vmo::PagedBackingStore, storage::fs::{VfsNode, VfsReadError}};

#[derive(Debug, Clone, Copy)]
pub enum FilePageWriteback {
    Page,
    TailBlock { offset: usize },
}

#[derive(Debug)]
struct CachedPage {
    frame: Arc<Frame4K>,
    writeback: Option<FilePageWriteback>,
}

#[derive(Debug)]
pub struct FileVmo {
    node: Weak<dyn VfsNode>,
    pages: TicketLock<BTreeMap<usize, CachedPage>>,
    pub io_lock: AsyncMutex<()>,
}

impl FileVmo {
    pub fn new(node: Weak<dyn VfsNode>) -> Arc<Self> {
        Arc::new(Self { node, pages: TicketLock::new(BTreeMap::new()), io_lock: AsyncMutex::new(()) })
    }

    pub fn cached_frame(&self, offset: usize) -> Option<Arc<Frame4K>> {
        self.pages.lock().get(&offset).map(|page| Arc::clone(&page.frame))
    }

    pub async fn get_frame_locked(&self, offset: usize, file_size: usize) -> Result<Arc<Frame4K>, VfsReadError> {
        if offset % PAGE_SIZE_4K != 0 {
            return Err(VfsReadError::InvalidArgument);
        }
        if let Some(frame) = self.cached_frame(offset) { return Ok(frame) };

        let frame = Arc::new(Frame4K::allocate_zeroed().ok_or(VfsReadError::OutOfMemory)?);
        let count = file_size.saturating_sub(offset).min(PAGE_SIZE_4K);

        if count != 0 {
            let node = self.node.upgrade().ok_or(VfsReadError::Io)?;
            let read = node.read_at_phys(offset, frame.phys(), count).await?;
            if read != count {
                return Err(VfsReadError::Io);
            }

            unsafe {
                (frame.virt() as *mut u8).add(count).write_bytes(0, PAGE_SIZE_4K - count);
            }
        }

        self.pages.lock().insert(offset, CachedPage { frame: Arc::clone(&frame), writeback: None });
        Ok(frame)
    }

    pub fn truncate_cached_locked(&self, new_size: usize, dirty_tail: Option<usize>) {
        let mut pages = self.pages.lock();
        pages.retain(|offset, _| *offset < new_size);

        let tail = new_size % PAGE_SIZE_4K;
        if tail != 0 {
            if let Some(page) = pages.get_mut(&(new_size - tail)) {
                unsafe { (page.frame.virt() as *mut u8).add(tail).write_bytes(0, PAGE_SIZE_4K - tail) };
                if !matches!(page.writeback, Some(FilePageWriteback::Page)) {
                    if let Some(offset) = dirty_tail {
                        page.writeback = Some(FilePageWriteback::TailBlock { offset });
                    } else if matches!(page.writeback, Some(FilePageWriteback::TailBlock { offset }) if offset >= tail) {
                        page.writeback = None;
                    }
                }
            }
        }
    }

    pub fn dirty_frames(&self) -> Vec<(usize, Arc<Frame4K>, FilePageWriteback)> {
        self.pages.lock().iter()
            .filter_map(|(&offset, page)| {
                page.writeback.map(|writeback| (offset, Arc::clone(&page.frame), writeback))
            })
            .collect()
    }

    pub fn mark_dirty(&self, offset: usize) -> Result<(), ()> {
        self.pages.lock().get_mut(&offset).ok_or(())?.writeback = Some(FilePageWriteback::Page);
        Ok(())
    }

    pub fn clear_dirty(&self, offset: usize) {
        if let Some(page) = self.pages.lock().get_mut(&offset) {
            page.writeback = None;
        }
    }

}

impl PagedBackingStore for FileVmo {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        block_on(Box::pin(async {
            let _guard = self.io_lock.lock().await;
            let node = self.node.upgrade().ok_or(())?;
            let size = node.size();
            if offset >= size { return Err(()) };

            let page_offset = offset - offset % PAGE_SIZE_4K;
            Ok(self.get_frame_locked(page_offset, size).await.map_err(|_| ())?.phys())
        }))
    }

    fn page_for_mapping(&self, _offset: usize) -> Result<MappingPage, ()> {
        Err(())
    }
}
