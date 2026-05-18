use core::alloc::{
    GlobalAlloc,
    Layout,
};
use core::ptr::null_mut;

use super::{
    ALLOCATOR,
    BlockSize,
    GLOBAL_VMM,
    HHDMOFFSET,
};
use crate::kernel::sync::TicketLock;
use crate::memory::pmm::{
    HUGE_PAGE_SIZE,
    NORMAL_PAGE_SIZE,
};
use crate::memory::vmm::{
    VM_FLAG_GLOBAL,
    VM_FLAG_HUGE,
    VM_FLAG_WRITE,
};

const CACHE_SIZES: [usize; 10] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

const KERNEL_VM_FLAGS: usize = VM_FLAG_WRITE | VM_FLAG_GLOBAL;

#[repr(C)]
struct FreeBlock {
    next: *mut FreeBlock,
}

pub struct KernelAllocator {
    caches: [TicketLock<KmemCache>; 10],
}

fn calc_size(layout: &Layout) -> usize { layout.size().max(layout.align()).next_power_of_two() }

impl KernelAllocator {
    pub const fn new() -> Self {
        let mut caches = [const { TicketLock::new(KmemCache { object_size: 0, freelist_head: None }) }; 10];
        let mut i = 0;
        while i < 10 {
            caches[i] = TicketLock::new(KmemCache { object_size: CACHE_SIZES[i], freelist_head: None });
            i += 1;
        }
        KernelAllocator { caches }
    }
}

unsafe impl Send for KernelAllocator {}
unsafe impl Sync for KernelAllocator {}

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = calc_size(&layout);
        if size <= NORMAL_PAGE_SIZE {
            // kmalloc
            let idx = size.ilog2().saturating_sub(3) as usize;
            let mut cache_lock = self.caches[idx].lock();
            unsafe { cache_lock.allocate() }
        } else {
            // vmalloc
            let mut vmm = GLOBAL_VMM.write();
            if size < HUGE_PAGE_SIZE {
                match vmm.mmap(size, KERNEL_VM_FLAGS) {
                    Some(addr) => addr as *mut u8,
                    None => null_mut(),
                }
            } else {
                match vmm.mmap(size, KERNEL_VM_FLAGS | VM_FLAG_HUGE) {
                    Some(addr) => addr as *mut u8,
                    None => null_mut(),
                }
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }
        let size = calc_size(&layout);

        if size <= NORMAL_PAGE_SIZE {
            // kfree
            let idx = size.ilog2().saturating_sub(3) as usize;
            let mut cache_lock = self.caches[idx].lock();
            unsafe { cache_lock.deallocate(ptr) }
        } else {
            // vfree
            let mut vmm = GLOBAL_VMM.write();
            let _ = vmm.munmap(ptr as usize, size);
        }
    }
}

pub struct KmemCache {
    object_size: usize,
    freelist_head: Option<*mut FreeBlock>,
}

impl KmemCache {
    pub unsafe fn allocate(&mut self) -> *mut u8 {
        if let Some(block) = self.freelist_head {
            if !block.is_null() {
                self.freelist_head = unsafe {
                    let next = (*block).next;
                    if next.is_null() { None } else { Some(next) }
                };
                return block as *mut u8;
            }
        }

        // separated slow path logic into separate function for clarity
        unsafe { self.refill() }
    }

    unsafe fn refill(&mut self) -> *mut u8 {
        let phys_addr = ALLOCATOR.alloc(BlockSize::Normal);

        let virt_page_start = phys_addr + *HHDMOFFSET;
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
            (*new_node).next = self.freelist_head.map_or(null_mut(), |h| h);
            self.freelist_head = Some(new_node);
        }
    }
}
