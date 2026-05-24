use core::alloc::{
    GlobalAlloc,
    Layout,
};
use crate::lock::TicketLock;
use core::ptr::null_mut;

pub trait PageProvider {
    fn allocate_pages(&self, size: usize) -> *mut u8;
    fn free_pages(&self, ptr: *mut u8, size: usize);
}

const CACHE_SIZES: [usize; 10] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

pub const NORMAL_PAGE_SIZE: usize = 4096;

#[repr(C)]
struct FreeBlock {
    next: *mut FreeBlock,
}

pub struct SlabAllocator<P: PageProvider> {
    caches: [TicketLock<MemCache>; 10],
    provider: P
}

fn calc_size(layout: &Layout) -> usize { layout.size().max(layout.align()).next_power_of_two() }

impl<P: PageProvider> SlabAllocator<P> {
    pub const fn new(provider: P) -> Self {
        let mut caches = [const { TicketLock::new(MemCache { object_size: 0, freelist_head: None }) }; 10];
        let mut i = 0;
        while i < 10 {
            caches[i] = TicketLock::new(MemCache { object_size: CACHE_SIZES[i], freelist_head: None });
            i += 1;
        }
        SlabAllocator { caches, provider }
    }
}

unsafe impl<P: PageProvider> Send for SlabAllocator<P> {}
unsafe impl<P: PageProvider> Sync for SlabAllocator<P> {}

unsafe impl<P: PageProvider> GlobalAlloc for SlabAllocator<P> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = calc_size(&layout);
        if size <= NORMAL_PAGE_SIZE {
            let idx = size.ilog2().saturating_sub(3) as usize;
            let mut cache_lock = self.caches[idx].lock();
            unsafe { cache_lock.allocate(&self.provider) }
        } else {
            // large allocations go directly to the provider bypassing memcache
            let pages = (size + NORMAL_PAGE_SIZE - 1) / NORMAL_PAGE_SIZE;
            let size_needed = pages * NORMAL_PAGE_SIZE;
            self.provider.allocate_pages(size_needed)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }
        let size = calc_size(&layout);
        if size <= NORMAL_PAGE_SIZE {
            let idx = size.ilog2().saturating_sub(3) as usize;
            let mut cache_lock = self.caches[idx].lock();
            unsafe { cache_lock.deallocate(ptr) }
        } else {
            let pages = (size + NORMAL_PAGE_SIZE - 1) / NORMAL_PAGE_SIZE;
            let size_needed = pages * NORMAL_PAGE_SIZE;
            self.provider.free_pages(ptr, size_needed);
        }
    }
}

pub struct MemCache {
    object_size: usize,
    freelist_head: Option<*mut FreeBlock>,
}

impl MemCache {
    pub unsafe fn allocate<P: PageProvider>(&mut self, provider: &P) -> *mut u8 {
        if let Some(block) = self.freelist_head {
            if !block.is_null() {
                self.freelist_head = unsafe {
                    let next = (*block).next;
                    if next.is_null() { None } else { Some(next) }
                };
                return block as *mut u8;
            }
        }

        unsafe { self.refill(provider) }
    }

    unsafe fn refill<P: PageProvider>(&mut self, provider: &P) -> *mut u8 {
        let virt_page_start = provider.allocate_pages(NORMAL_PAGE_SIZE) as usize;
        let num_objects = NORMAL_PAGE_SIZE / self.object_size;

        for i in 1..num_objects {
            let current_ptr = (virt_page_start + (self.object_size * i)) as *mut FreeBlock;
            let next_ptr =
                if i == num_objects - 1 { null_mut() } else { (virt_page_start + ((i + 1) * self.object_size)) as *mut FreeBlock };

            unsafe {
                (*current_ptr).next = next_ptr;
            }
        }

        if num_objects > 1 {
            self.freelist_head = Some((virt_page_start + self.object_size) as *mut FreeBlock);
        } else {
            self.freelist_head = None;
        }

        virt_page_start as *mut u8
    }

    pub unsafe fn deallocate(&mut self, ptr: *mut u8) {
        unsafe {
            let new_node = ptr as *mut FreeBlock;
            (*new_node).next = self.freelist_head.unwrap_or(null_mut());
            self.freelist_head = Some(new_node);
        }
    }
}
