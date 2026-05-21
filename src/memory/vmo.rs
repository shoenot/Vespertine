use core::fmt::Debug;

use alloc::{collections::btree_map::BTreeMap, sync::Arc};

use crate::{kernel::sync::TicketLock, memory::{ALLOCATOR, GLOBAL_PMM, PCAllocator, pmm::NORMAL_PAGE_SIZE}};

#[derive(Debug)]
pub struct Vmo {
    pub size: usize,
    pub pages: TicketLock<BTreeMap<usize, usize>>,
}

pub trait PagedBackingStore: Send + Sync + Debug {
    fn request_page(&self, offset: usize) -> Result<usize, ()>;
    fn resize_object(&self, new_size: usize) -> Result<(), ()>;
}

impl PagedBackingStore for Vmo {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        let mut pages = self.pages.lock();

        if offset >= self.size {
            return Err(())
        }

        if let Some(&pfn) = pages.get(&offset) {
            if pfn != 0 { return Ok(pfn) };
        }

        // allocate directly from the pmm 
        let pfn = ALLOCATOR.alloc(super::BlockSize::Normal);

        if let Some(entry) = pages.get_mut(&offset) {
            *entry = pfn;
        }

        Ok(pfn as usize)
    }

    fn resize_object(&self, new_size: usize) -> Result<(), ()> {
        Ok(())
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
            size, 
            pages: TicketLock::new(pages),
        })
    }

    pub fn resize(&self, new_size: usize) {
        unimplemented!()
    }

    pub fn clone_range(&self, offset: usize, len: usize) -> usize {
        unimplemented!()
    }

    // for demand paging
    pub fn get_page(&self, offset: usize) -> usize {
        let mut pages = self.pages.lock();

        // return page if its already allocated
        if let Some(&pfn) = pages.get(&offset) {
            if pfn != 0 {
                return pfn;
            }
        } else {
            panic!("OOB Vmo access at offset {}", offset);
        }

        let pfn = GLOBAL_PMM.lock().alloc(super::BlockSize::Normal)
            .expect("Out of physical memory!");

        pages.insert(offset, pfn);
        pfn
    }
}
