use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::fmt::Debug;
use core::ptr::{copy_nonoverlapping, write_bytes};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::core::asynchronous::syscall_bridge::block_on;
use crate::core::sync::TicketLock;
use crate::storage::fs::VfsNode;
use crate::memory::pmm::{
    NORMAL_PAGE_SIZE,
    PF_PINNED,
};
use crate::memory::{
    ALLOCATOR,
    BlockSize,
    GLOBAL_PMM,
    HHDMOFFSET,
};

#[derive(Debug)]
pub struct Vmo {
    pub size: AtomicUsize,
    pub pages: TicketLock<BTreeMap<usize, usize>>,
    pub is_physical: bool,
}

pub trait PagedBackingStore: Send + Sync + Debug {
    fn request_page(&self, offset: usize) -> Result<usize, ()>;
    fn resize_object(&self, new_size: usize) -> Result<(), ()>;
    fn clone_range(&self, offset: usize, len: usize) -> Result<Arc<dyn PagedBackingStore>, ()>;
    fn pin(self: Arc<Self>, offset: usize, len: usize) -> Result<PinnedVmo, ()>;
}

impl PagedBackingStore for Vmo {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        let mut pages = self.pages.lock();

        let current_size = self.size.load(Ordering::Relaxed);
        if offset >= current_size {
            return Err(());
        }

        if let Some(&pfn) = pages.get(&offset) {
            if pfn != 0 {
                return Ok(pfn);
            };
        }

        if self.is_physical {
            return Err(());
        }

        // allocate directly from the pmm
        let pfn = ALLOCATOR.alloc(BlockSize::Normal);
        pages.insert(offset, pfn);
        Ok(pfn as usize)
    }

    fn resize_object(&self, new_size: usize) -> Result<(), ()> {
        if self.is_physical {
            return Err(());
        }
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
        if self.is_physical {
            return Err(());
        }
        let pages = self.pages.lock();
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
        Ok(Arc::new(Vmo { size: AtomicUsize::new(len), pages: TicketLock::new(child_pages), is_physical: false }))
    }

    fn pin(self: Arc<Self>, offset: usize, len: usize) -> Result<PinnedVmo, ()> {
        let current_size = self.size.load(Ordering::Relaxed);
        if offset + len > current_size {
            return Err(());
        }

        let start_page = offset / NORMAL_PAGE_SIZE;
        let end_page = (offset + len).div_ceil(NORMAL_PAGE_SIZE);
        let mut phys_addrs = Vec::new();

        for i in start_page..end_page {
            let page_offset = i * NORMAL_PAGE_SIZE;
            let addr = self.request_page(page_offset)?;
            phys_addrs.push(addr);
        }

        let pmm = GLOBAL_PMM.lock();
        for &addr in &phys_addrs {
            let pfn = addr / NORMAL_PAGE_SIZE;
            if pfn < pmm.pfndb.len() {
                pmm.pfndb[pfn].flags.fetch_or(PF_PINNED, Ordering::SeqCst);
            }
        }
        Ok(PinnedVmo { vmo: self, phys_addrs })
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

        Arc::new(Self { size: AtomicUsize::new(size), pages: TicketLock::new(pages), is_physical: false })
    }

    pub fn new_phys(phys_addr: usize, size: usize) -> Arc<Self> {
        let mut pages = BTreeMap::new();
        let num_pages = size.div_ceil(NORMAL_PAGE_SIZE);
        for i in 0..num_pages {
            let offset = i * NORMAL_PAGE_SIZE;
            pages.insert(offset, phys_addr + offset);
        }

        Arc::new(Self { size: AtomicUsize::new(size), pages: TicketLock::new(pages), is_physical: true })
    }
}

impl Drop for Vmo {
    fn drop(&mut self) {
        if self.is_physical {
            return;
        }

        let pages = self.pages.lock();
        for (&_offset, &pfn) in pages.iter() {
            if pfn != 0 {
                ALLOCATOR.free(pfn, BlockSize::Normal);
            }
        }
    }
}

#[derive(Debug)]
pub struct PinnedVmo {
    vmo: Arc<dyn PagedBackingStore>,
    phys_addrs: Vec<usize>,
}

impl PinnedVmo {
    pub fn phys_addrs(&self) -> &[usize] { &self.phys_addrs }
}

impl Drop for PinnedVmo {
    fn drop(&mut self) {
        let pmm = GLOBAL_PMM.lock();

        for &addr in &self.phys_addrs {
            let pfn = addr / NORMAL_PAGE_SIZE;
            if pfn < pmm.pfndb.len() {
                // clear the pf pinned flag
                pmm.pfndb[pfn].flags.fetch_and(!PF_PINNED, Ordering::SeqCst);
            }
        }
    }
}

#[derive(Debug)]
pub struct FileVmo {
    pub anonymous_vmo: Arc<Vmo>,
    pub node: Weak<dyn VfsNode>,
}

impl FileVmo {
    pub fn new(size: usize, node: Weak<dyn VfsNode>) -> Arc<Self> {
        Arc::new(Self { 
            anonymous_vmo: Vmo::new(size), 
            node,
        })
    }
}

impl PagedBackingStore for FileVmo {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        // check if page alr loaded in ram
        let mut pages = self.anonymous_vmo.pages.lock();

        let current_size = self.anonymous_vmo.size.load(Ordering::Relaxed);
        if offset >= current_size {
            return Err(());
        }

        if let Some(&pfn) = pages.get(&offset) {
            if pfn != 0 {
                return Ok(pfn);
            }
        }

        // cache miss
        let page_phys = ALLOCATOR.alloc(BlockSize::Normal) as usize;
        if page_phys == 0 {
            return Err(());
        }

        let node = self.node.upgrade().ok_or(())?;
        let read_fut = node.read_at_phys(offset, page_phys, NORMAL_PAGE_SIZE);
        let bytes_read = block_on(Box::pin(read_fut)).map_err(|_| ())?; 

        if bytes_read < NORMAL_PAGE_SIZE {
            unsafe {
                let dest_virt = page_phys + bytes_read + *HHDMOFFSET;
                write_bytes(dest_virt as *mut u8, 0, NORMAL_PAGE_SIZE - bytes_read);
            }
        }

        pages.insert(offset, page_phys);
        Ok(page_phys)
    }

    fn resize_object(&self, new_size: usize) -> Result<(), ()> { 
        let node = self.node.upgrade().ok_or(())?;
        node.resize(new_size)?;
        self.anonymous_vmo.resize_object(new_size)
    }

    fn clone_range(&self, offset: usize, len: usize) -> Result<Arc<dyn PagedBackingStore>, ()> {
        self.anonymous_vmo.clone_range(offset, len)
    }

    fn pin(self: Arc<Self>, offset: usize, len: usize) -> Result<PinnedVmo, ()> {
        let current_size = self.anonymous_vmo.size.load(Ordering::Relaxed);
        if offset + len > current_size {
            return Err(());
        }

        let start_page = offset / NORMAL_PAGE_SIZE;
        let end_page = (offset + len).div_ceil(NORMAL_PAGE_SIZE);
        let mut phys_addrs = Vec::new();

        // ensure all pages are faulted/loaded
        for i in start_page..end_page {
            let page_offset = i * NORMAL_PAGE_SIZE;
            let addr = self.request_page(page_offset)?;
            phys_addrs.push(addr);
        }

        // pin pages in the pmm so they cant be reclaimed
        let pmm = GLOBAL_PMM.lock();
        for &addr in &phys_addrs {
            let pfn = addr / NORMAL_PAGE_SIZE;
            if pfn < pmm.pfndb.len() {
                pmm.pfndb[pfn].flags.fetch_or(PF_PINNED, Ordering::SeqCst);
            }
        }

        Ok(PinnedVmo { vmo: self, phys_addrs })
    }
}
