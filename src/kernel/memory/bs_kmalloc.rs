use core::ptr;
use lazy_static::lazy_static;
use crate::TicketLock;

pub struct BumpAllocator {
    heap_start: usize,
    heap_end: usize,
    next: usize,
}

lazy_static!(
    static ref BOOTSTRAP_BUFFER: [u8; 128 * 1024] = [0; 128 * 1024];
);

pub static BOOTSTRAP_ALLOC: TicketLock<BumpAllocator> = TicketLock::new(BumpAllocator::new());

impl BumpAllocator {
    pub const fn new() -> Self {
        BumpAllocator { heap_start: 0, heap_end: 0, next: 0 }
    }

    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.heap_start = heap_start;
        self.heap_end = heap_start + heap_size;
        self.next = heap_start;
    }

    pub fn alloc(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let alloc_start = (self.next + align - 1) & !(align - 1);
        let alloc_end = alloc_start.saturating_add(size);

        if alloc_end <= self.heap_end {
            self.next = alloc_end;
            Some(alloc_start as *mut u8)
        } else {
            None
        }
    }

    pub fn free(&mut self, _ptr: *mut u8, _size: usize) {}
}

pub fn init_bootstrap_allocator() {
    unsafe {
        let start = BOOTSTRAP_BUFFER.as_ptr() as usize;
        let size = BOOTSTRAP_BUFFER.len();
        BOOTSTRAP_ALLOC.lock().init(start, size);
    }
}
