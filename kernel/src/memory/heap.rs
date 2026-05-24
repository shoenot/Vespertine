use core::ptr::null_mut;

use mnemosyne_common::slab::{PageProvider, SlabAllocator};
use crate::memory::{ALLOCATOR, HHDMOFFSET, BlockSize};

pub struct KernelPageProvider;

impl PageProvider for KernelPageProvider {
    fn allocate_pages(&self, size: usize) -> *mut u8 {
        // slab page reqs are normalized to 4kb
        if size <= 4096 {
            let phys = ALLOCATOR.alloc(BlockSize::Normal);
            return (phys + *HHDMOFFSET) as *mut u8;
        } else {
            // large allocations go directly to the buddy 
            let pages = (size + 4095) / 4096;
            let order = pages.next_power_of_two().trailing_zeros() as usize;
            if let Some(phys) = ALLOCATOR.alloc_order(order) {
                return (phys + *HHDMOFFSET) as *mut u8;
            }
        }
        null_mut()
    }

    fn free_pages(&self, ptr: *mut u8, size: usize) {
        if ptr.is_null() {
            return;
        }
        let phys = ptr as usize - *HHDMOFFSET;
        if size <= 4096 {
            ALLOCATOR.free(phys, BlockSize::Normal);
        } else {
            let pages = (size + 4095) / 4096;
            let order = pages.next_power_of_two().trailing_zeros() as usize;
            ALLOCATOR.free_order(phys, order);
        }
    }
}

pub type KernelAllocator = SlabAllocator<KernelPageProvider>;
