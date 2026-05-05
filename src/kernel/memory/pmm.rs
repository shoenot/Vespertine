use core::slice::from_raw_parts_mut;
use crate::{drivers::serial::*, kernel::memory::init_pmm::*};

static ORDER_MAX: usize = 11;
static PAGE_SIZE: usize = 4096;

#[repr(C)]
struct FreeBlock {
    prev: usize,
    next: usize,
}

pub struct Allocator {
    freelist: [usize; ORDER_MAX + 1],
    metadata: &'static mut [u8],
    pub metadata_phys_addr: usize,
    meta_bit_offset: [usize; ORDER_MAX + 1],
}

impl Allocator {
    pub fn init() -> Self {
        let mut init_allocator = BitmapPMM::init();
        let max_addr = init_allocator.max_addr;
        let metadata_size = max_addr / (PAGE_SIZE * 8);
        let meta_frame_idx = init_allocator.alloc(metadata_size).unwrap();
        let meta_phys = meta_frame_idx * PAGE_SIZE;
        let meta_virt = meta_phys + *HHDMOFFSET;

        let metadata_slice = unsafe { from_raw_parts_mut(meta_virt as *mut u8, metadata_size) };

        metadata_slice.fill(0);

        let total_pages = max_addr / PAGE_SIZE;
        let mut meta_bit_offset = [0; ORDER_MAX + 1];

        let mut current_offset = 0;
        for order in 0..(ORDER_MAX+1) {
            meta_bit_offset[order] = current_offset;
            current_offset += (total_pages >> order) / 2;
        }

        let mut allocator = Allocator { 
            freelist: [0; ORDER_MAX + 1], 
            metadata: metadata_slice, 
            metadata_phys_addr: meta_phys,
            meta_bit_offset,
        };

        for frame in 0..init_allocator.total_frames {
            if init_allocator.is_free(frame) {
                let frame_addr = frame * PAGE_SIZE;
                allocator.free(frame_addr, 0);
            }
        }
        
        log_to_serial("Primary Allocator metadata stored at ");
        log_u64_to_serial(meta_phys as u64);

        allocator
    }

    pub fn alloc(&mut self, mut order: usize) -> Option<usize> {
        let target_order = order;
        let block_addr = { 
            loop {
                let list_addr = self.freelist[order];

                if list_addr == 0 { 
                    order += 1;  
                    if order > ORDER_MAX { return None } else { continue }
                }

                let list_ptr = (list_addr + *HHDMOFFSET) as *mut FreeBlock;
                let block = unsafe{ &mut *list_ptr };

                if block.next != 0 {
                    let next_block = unsafe { let blk = (block.next + *HHDMOFFSET) as *mut FreeBlock; &mut *blk };
                    next_block.prev = 0;
                }

                self.freelist[order] = block.next;

                break list_addr;
            }
        };

        self.split_block(block_addr, order, target_order);

        Some(block_addr)
    }

    fn split_block(&mut self, addr: usize, mut order: usize, target_order: usize) {
        while order > target_order {
            order -= 1;

            let buddy_addr = addr + (PAGE_SIZE << order);
            let buddy_ptr = (buddy_addr + *HHDMOFFSET) as *mut FreeBlock;

            let order_head_addr = self.freelist[order];
            if order_head_addr != 0 {
                let order_head_ptr = unsafe { let blk = (self.freelist[order] + *HHDMOFFSET) as *mut FreeBlock; &mut *blk };
                order_head_ptr.prev = buddy_addr;
            }

            self.freelist[order] = buddy_addr;

            unsafe {
                *buddy_ptr = FreeBlock { prev: 0, next: order_head_addr };
            }

            // bitmap shit 
            let page_idx = buddy_addr / PAGE_SIZE;
            let pair_idx = page_idx >> (order + 1);
            let abs_bit = self.meta_bit_offset[order] + pair_idx;
            let byte_idx = abs_bit / 8;
            let bit_idx = abs_bit % 8;
            self.metadata[byte_idx] ^= 1 << bit_idx;
        }
    }

    fn xor_bit(&mut self, block_addr: usize, order: usize) -> bool {
        let page_idx = block_addr / PAGE_SIZE;
        let pair_idx = page_idx >> (order + 1);
        let abs_bit = self.meta_bit_offset[order] + pair_idx;
        let byte_idx = abs_bit / 8;
        let bit_idx = abs_bit % 8;
        self.metadata[byte_idx] ^= 1 << bit_idx;
        if self.metadata[byte_idx] & (1 << bit_idx) == 0 { true } else { false }
    }

    pub fn free(&mut self, mut block_addr: usize, mut order: usize) {
        while self.xor_bit(block_addr, order) && order < ORDER_MAX {
            let block_size = PAGE_SIZE << order;
            let buddy_addr = block_addr ^ block_size;
            block_addr = self.merge_block(block_addr, buddy_addr, order);
            order += 1;
        }
        
        // add block 
        let new_block_ptr = (block_addr + *HHDMOFFSET) as *mut FreeBlock;
        unsafe {
            let old_head = self.freelist[order];
            *new_block_ptr = FreeBlock { prev: 0, next: self.freelist[order] };

            if old_head != 0 {
                let old_head_ptr = (old_head + *HHDMOFFSET) as *mut FreeBlock;
                (*old_head_ptr).prev = block_addr;
            }
        }
        self.freelist[order] = block_addr;
    }

    fn merge_block(&mut self, block_addr: usize, buddy_addr: usize, order: usize) -> usize {
        let buddy_ptr = (buddy_addr + *HHDMOFFSET) as *mut FreeBlock;
        unsafe {
            let prev = (*buddy_ptr).prev;
            let next = (*buddy_ptr).next;

            if prev != 0 {
                let prev_ptr = (prev + *HHDMOFFSET) as *mut FreeBlock;
                (*prev_ptr).next = next;
            } else {
                self.freelist[order] = next;
            }
            
            if next != 0 {
                let next_ptr = (next + *HHDMOFFSET) as *mut FreeBlock;
                (*next_ptr).prev = prev;
            }

            (*buddy_ptr).prev = 0;
            (*buddy_ptr).next = 0;

        }
        
        // return left addr  
        if block_addr < buddy_addr { block_addr } else { buddy_addr }
    }
}
