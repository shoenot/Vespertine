use core::slice::from_raw_parts_mut;
use crate::memory::init_pmm::BitmapPMM;
pub use crate::memory::HHDMOFFSET;


static ORDER_MAX: usize = 11;
pub static HUGE_PAGE_SIZE: usize = 0x20_0000;
pub static NORMAL_PAGE_SIZE: usize = 0x1000;

#[repr(C)]
struct FreeBlock {
    prev: usize,
    next: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum BlockSize {
    Normal,
    Huge,
}

pub struct Allocator {
    freelist: [usize; ORDER_MAX + 1],
    metadata: &'static mut [u8],
    pub metadata_phys_addr: usize,
    meta_bit_offset: [usize; ORDER_MAX + 1],
}

impl Allocator {
    pub const fn new() -> Self {
        Self {
            freelist: [0; ORDER_MAX + 1],
            metadata: &mut [],
            metadata_phys_addr: 0,
            meta_bit_offset: [0; ORDER_MAX + 1],
        }
    }

    pub fn init(&mut self) {
        let mut init_allocator = BitmapPMM::init();
        let max_addr = init_allocator.max_addr;
        let metadata_size = max_addr / (NORMAL_PAGE_SIZE * 8);
        let meta_frame_idx = init_allocator.alloc(metadata_size).unwrap();
        let meta_phys = meta_frame_idx * NORMAL_PAGE_SIZE;
        let meta_virt = meta_phys + *HHDMOFFSET;

        let metadata_slice = unsafe { from_raw_parts_mut(meta_virt as *mut u8, metadata_size) };

        metadata_slice.fill(0);

        let total_pages = max_addr / NORMAL_PAGE_SIZE;
        let mut meta_bit_offset = [0; ORDER_MAX + 1];

        let mut current_offset = 0;
        for order in 0..(ORDER_MAX+1) {
            meta_bit_offset[order] = current_offset;
            current_offset += (total_pages >> order) / 2;
        }

        self.freelist = [0; ORDER_MAX + 1];
        self.metadata = metadata_slice;
        self.metadata_phys_addr = meta_phys;
        self.meta_bit_offset = meta_bit_offset;

        let mut cursor = 0;
        while cursor < init_allocator.total_frames {
            while cursor < init_allocator.total_frames && !init_allocator.is_free(cursor) { 
                cursor += 1;  // skip used frames
            } 
            if cursor >= init_allocator.total_frames { break; }
            let mut start_frame = cursor;
            while cursor < init_allocator.total_frames && init_allocator.is_free(cursor)  { cursor += 1; }
            let end_frame = cursor;

            loop {
                let mut max_order = start_frame.trailing_zeros() as usize; // max order allowed based on alignment constraints
                max_order = if max_order > ORDER_MAX { ORDER_MAX } else { max_order };

                let mut order = max_order;
                loop {
                    let frames_needed = 1 << order;
                    if frames_needed > (end_frame - start_frame) { 
                        order -= 1; continue; 
                    } else {
                        self.free_order(start_frame * NORMAL_PAGE_SIZE, order);
                        start_frame += 1 << order;
                        break;
                    }
                }
                if start_frame >= end_frame { break; }
            }
        }
    }

    pub fn alloc(&mut self, size: BlockSize) -> Option<usize> {
        let order = match size {
            BlockSize::Huge => 9,
            BlockSize::Normal => 0,
        };
        self.alloc_order(order)
    }

    pub fn alloc_order(&mut self, mut order: usize) -> Option<usize> {
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

        self.xor_bit(block_addr, order);

        self.split_block(block_addr, order, target_order);

        Some(block_addr)
    }

    fn split_block(&mut self, addr: usize, mut order: usize, target_order: usize) {
        while order > target_order {
            order -= 1;

            let buddy_addr = addr + (NORMAL_PAGE_SIZE << order);
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
            let page_idx = buddy_addr / NORMAL_PAGE_SIZE;
            let pair_idx = page_idx >> (order + 1);
            let abs_bit = self.meta_bit_offset[order] + pair_idx;
            let byte_idx = abs_bit / 8;
            let bit_idx = abs_bit % 8;
            self.metadata[byte_idx] ^= 1 << bit_idx;
        }
    }

    fn xor_bit(&mut self, block_addr: usize, order: usize) -> bool {
        let page_idx = block_addr / NORMAL_PAGE_SIZE;
        let pair_idx = page_idx >> (order + 1);
        let abs_bit = self.meta_bit_offset[order] + pair_idx;
        let byte_idx = abs_bit / 8;
        let bit_idx = abs_bit % 8;
        self.metadata[byte_idx] ^= 1 << bit_idx;
        if self.metadata[byte_idx] & (1 << bit_idx) == 0 { true } else { false }
    }

    pub fn free(&mut self, block_addr: usize, size: BlockSize) {
        let order = match size {
            BlockSize::Huge => 9,
            BlockSize::Normal => 0,
        };
        self.free_order(block_addr, order);
    }

    pub fn free_order(&mut self, mut block_addr: usize, mut order: usize) {
        while self.xor_bit(block_addr, order) && order < ORDER_MAX {
            let block_size = NORMAL_PAGE_SIZE << order;
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
