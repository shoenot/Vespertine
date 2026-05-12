use core::ops::Deref;

use lazy_static::lazy_static;
use limine::memmap::*;

use crate::{
    HHDM_REQUEST,
    MEMMAP_REQUEST, kernel::sync::KernelOnceCell,
};

lazy_static! {
    pub static ref HHDMOFFSET: usize = if let Some(hhdmresp) = HHDM_REQUEST.response() {
        hhdmresp.deref().offset as usize
    } else {
        panic!("COULD NOT GET HHDM OFFSET FROM LIMINE")
    };
}

pub static HHDMOFFSET: KernelOnceCell<usize> = KernelOnceCell::new();

pub static HUGE_PAGE_SIZE: usize = 0x20_0000;
pub static NORMAL_PAGE_SIZE: usize = 0x1000;

#[derive(Clone, Copy, PartialEq)]
pub enum BlockSize {
    Normal,
    Huge,
}

#[repr(C)]
pub struct FreeBlock {
    next: usize,
    size: BlockSize,
}

pub struct Allocator {
    pub normal_head: usize,
    pub huge_head: usize,
    pub highest_addr: usize,
    pub free_4k: usize,
    pub free_2m: usize,
}

fn get_start_end(base: usize, length: usize) -> (usize, usize) {
    let start = (base + 0xFFF) & !0xFFF;
    let end = (base + length) & !0xFFF;
    (start, end)
}

fn get_start_end_huge(base: usize, length: usize) -> (usize, usize) {
    let start = (base + 0x1F_FFFF) & !0x1F_FFFF;
    let end = (base + length) & !0x1F_FFFF;
    (start, end)
}

fn get_block(addr: usize) -> &'static mut FreeBlock {
    unsafe {
        let block = &mut *((addr + *HHDMOFFSET) as *mut FreeBlock);
        &mut (*block)
    }
}

fn init_pages(mut start: usize, mut limit: usize, head: &mut usize, size: BlockSize) -> usize {
    let mut alloc_count = 0;
    let spacing = if size == BlockSize::Normal { NORMAL_PAGE_SIZE } else { HUGE_PAGE_SIZE };
    while limit > 0 {
        let block = get_block(start);
        *block = FreeBlock { next: *head, size };
        *head = start;
        start += spacing;
        limit -= 1;
        alloc_count += 1;
    }
    alloc_count
}

impl Allocator {
    pub const fn new() -> Self { Allocator { normal_head: 0, huge_head: 0, highest_addr: 0, free_4k: 0, free_2m: 0 } }

    pub fn init(&mut self) {
        HHDMOFFSET.get_or_init(|| {
            HHDM_REQUEST.response().expect("Failed to get HHDM offset from Limine").offset as usize
        });

        let mem_map = if let Some(memmap_response) = MEMMAP_REQUEST.response() {
            memmap_response.deref().entries()
        } else {
            panic!("COULD NOT GET MEMMAP FROM LIMINE")
        };

        for entry in mem_map {
            let top = entry.base + entry.length;
            if top as usize > self.highest_addr {
                self.highest_addr = top as usize;
            }
        }

        self.highest_addr = (self.highest_addr + 4095) & !4095;

        for entry in mem_map {
            if entry.type_ == MEMMAP_USABLE {
                let (start_4k, end_4k) = get_start_end(entry.base as usize, entry.length as usize);
                let (start_2m, end_2m) = get_start_end_huge(entry.base as usize, entry.length as usize);

                // sometimes chunks can't fit a full 2 megs
                if start_2m < end_2m {
                    let fgap_4k_pages = (start_2m - start_4k) / NORMAL_PAGE_SIZE;
                    self.free_4k += init_pages(start_4k, fgap_4k_pages, &mut self.normal_head, BlockSize::Normal);

                    let middle_pages = (end_2m - start_2m) / HUGE_PAGE_SIZE;
                    self.free_2m += init_pages(start_2m, middle_pages, &mut self.huge_head, BlockSize::Huge);

                    let egap_4k_pages = (end_4k - end_2m) / NORMAL_PAGE_SIZE;
                    self.free_4k += init_pages(end_2m, egap_4k_pages, &mut self.normal_head, BlockSize::Normal);
                } else {
                    let pages = (end_4k - start_4k) / NORMAL_PAGE_SIZE;
                    self.free_4k += init_pages(start_4k, pages, &mut self.normal_head, BlockSize::Normal);
                }
            }
        }
    }

    pub fn alloc(&mut self, size: BlockSize) -> Option<usize> {
        match size {
            BlockSize::Normal => {
                if self.free_4k > 0 {
                    self.pop(BlockSize::Normal)
                } else if self.free_2m > 0 {
                    self.split_huge()
                } else {
                    None
                }
            }
            BlockSize::Huge => {
                if self.free_2m > 0 {
                    self.pop(BlockSize::Huge)
                } else {
                    None
                }
            }
        }
    }

    pub fn free(&mut self, addr: usize, size: BlockSize) { self.push(size, addr); }

    fn pop(&mut self, size: BlockSize) -> Option<usize> {
        match size {
            BlockSize::Normal => {
                if self.normal_head == 0 {
                    return None;
                };
                let ret = self.normal_head;
                let next_addr = unsafe {
                    let blk = (ret + *HHDMOFFSET) as *const FreeBlock;
                    &*blk
                }
                .next;
                self.normal_head = next_addr;
                self.free_4k -= 1;
                Some(ret)
            }
            BlockSize::Huge => {
                if self.huge_head == 0 {
                    return None;
                };
                let ret = self.huge_head;
                let next_addr = unsafe {
                    let blk = (ret + *HHDMOFFSET) as *const FreeBlock;
                    &*blk
                }
                .next;
                self.huge_head = next_addr;
                self.free_2m -= 1;
                Some(ret)
            }
        }
    }

    fn push(&mut self, size: BlockSize, addr: usize) {
        match size {
            BlockSize::Normal => {
                let next = self.normal_head;
                let new_block = unsafe {
                    let blk = (addr + *HHDMOFFSET) as *mut FreeBlock;
                    &mut *blk
                };
                *new_block = FreeBlock { next, size };
                self.normal_head = addr;
                self.free_4k += 1;
            }
            BlockSize::Huge => {
                let next = self.huge_head;
                let new_block = unsafe {
                    let blk = (addr + *HHDMOFFSET) as *mut FreeBlock;
                    &mut *blk
                };
                *new_block = FreeBlock { next, size };
                self.huge_head = addr;
                self.free_2m += 1;
            }
        }
    }

    fn split_huge(&mut self) -> Option<usize> {
        if self.huge_head == 0 {
            return None;
        };
        let block = self.pop(BlockSize::Huge)?;
        for i in 0..512 {
            let base = block + (i << 12);
            let new_block = unsafe {
                let blk = (base + *HHDMOFFSET) as *mut FreeBlock;
                &mut *blk
            };
            *new_block = FreeBlock { next: self.normal_head, size: BlockSize::Normal };
            self.normal_head = base;
        }
        self.free_4k += 512;
        self.pop(BlockSize::Normal)
    }
}
