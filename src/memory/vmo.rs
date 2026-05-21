use core::{fmt::Debug, intrinsics::copy_nonoverlapping, sync::atomic::{AtomicUsize, Ordering}};

use alloc::{collections::btree_map::BTreeMap, sync::Arc, vec::Vec};

use crate::{kernel::sync::TicketLock, memory::{ALLOCATOR, BlockSize, HHDMOFFSET, pmm::NORMAL_PAGE_SIZE}};

#[derive(Debug)]
pub struct Vmo {
    pub size: AtomicUsize,
    pub pages: TicketLock<BTreeMap<usize, usize>>,
}

pub trait PagedBackingStore: Send + Sync + Debug {
    fn request_page(&self, offset: usize) -> Result<usize, ()>;
    fn resize_object(&self, new_size: usize) -> Result<(), ()>;
    fn clone_range(&self, offset: usize, len: usize) -> Result<Arc<dyn PagedBackingStore>, ()>;
}

impl PagedBackingStore for Vmo {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        let mut pages = self.pages.lock();

        let current_size = self.size.load(Ordering::Relaxed);
        if offset >= current_size {
            return Err(())
        }

        if let Some(&pfn) = pages.get(&offset) {
            if pfn != 0 { return Ok(pfn) };
        }

        // allocate directly from the pmm 
        let pfn = ALLOCATOR.alloc(BlockSize::Normal);
        pages.insert(offset, pfn);
        Ok(pfn as usize)
    }

    fn resize_object(&self, new_size: usize) -> Result<(), ()> {
        let mut pages = self.pages.lock();
        let old_size = self.size.load(Ordering::Relaxed);

        if new_size == old_size {
            return Ok(());
        }

        if new_size < old_size {
            // shrink, free pages beyond new size
            let mut to_remove = Vec::new();
            for (&offset, &pfn) in pages.iter() {
                if offset >= new_size {
                    if pfn != 0 {
                        ALLOCATOR.free(pfn, BlockSize::Normal);
                    }
                    to_remove.push(offset);
                }
            }
            for offset in to_remove {
                pages.remove(&offset);
            }
        } else {
            // grow, pad map with 0s
            let num_pages = new_size.div_ceil(NORMAL_PAGE_SIZE);
            for i in 0..num_pages {
                let offset = i * NORMAL_PAGE_SIZE;
                pages.entry(offset).or_insert(0);
            }       
        }
        self.size.store(new_size, Ordering::Relaxed);
        Ok(())
    }

    fn clone_range(&self, offset: usize, len: usize) -> Result<Arc<dyn PagedBackingStore>, ()> {
        let mut pages = self.pages.lock();
        let current_size = self.size.load(Ordering::Relaxed);

        if offset + len > current_size {
            return Err(());
        }

        let mut child_pages = BTreeMap::new();
        let num_pages = len.div_ceil(NORMAL_PAGE_SIZE);

        for i in 0..num_pages {
            let page_offset = i * NORMAL_PAGE_SIZE;
            let parent_offset = offset + page_offset;

            let child_pfn = ALLOCATOR.alloc(BlockSize::Normal);

            // copy from parent to child if parent was alr allocated. can skip if no 
            if let Some(&parent_pfn) = pages.get(&parent_offset) {
                if parent_pfn != 0 {
                    let parent_virt = parent_pfn + *HHDMOFFSET;
                    let child_virt = child_pfn + *HHDMOFFSET;
                    unsafe {
                        copy_nonoverlapping(parent_virt as *mut u8, child_virt as *mut u8, NORMAL_PAGE_SIZE);
                    }
                }
            }
            child_pages.insert(page_offset, child_pfn);
        }
        Ok(Arc::new(Vmo {
            size: AtomicUsize::new(len),
            pages: TicketLock::new(child_pages),
        }))
    }
}


impl Vmo {
    pub fn new(size: usize) -> Arc<Self> {
        let mut pages = BTreeMap::new();
        let num_pages = size.div_ceil(NORMAL_PAGE_SIZE);
        for i in 0..num_pages {
            let offset = i * NORMAL_PAGE_SIZE;
            pages.insert(offset, 0);
        }

        Arc::new(Self {
            size: AtomicUsize::new(size), 
            pages: TicketLock::new(pages),
        })
    }
}
