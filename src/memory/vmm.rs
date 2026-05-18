#![allow(dead_code)]

use alloc::alloc::{
    Layout,
    alloc,
    dealloc,
};

// TODO: Optimize VMM tlb shootdowns. make it loop and unmap all the pages first and *then* fire the
// ipis.
use super::paging::*;
use super::pmm::*;
use crate::arch::x86_64::interrupts::shootdown::shootdown;
use crate::kernel::sync::TicketLock;
use crate::memory::PCAllocator;

pub static VM_FLAG_WRITE: usize = 1 << 0;
pub static VM_FLAG_EXEC: usize = 1 << 1;
pub static VM_FLAG_USER: usize = 1 << 2;
pub static VM_FLAG_HUGE: usize = 1 << 3;
pub static VM_FLAG_GLOBAL: usize = 1 << 4;
pub static VM_FLAG_CACHE_DISABLE: usize = 1 << 5;
pub static VM_FLAG_WRITE_THROUGH: usize = 1 << 5;

static VM_BASE_ADDR: usize = 0x4000_0000;
static VM_MAX_ALLOWED: usize = 0x0000_7FFF_FFFF_F000;

fn convert_vm_flags(flags: usize) -> usize {
    let mut writable = false;
    let mut user_access = false;
    let mut global = false;
    let mut no_execute = true;
    let mut cache_disable = false;
    let mut write_through = false;
    if flags & VM_FLAG_WRITE != 0 {
        writable = true
    };
    if flags & VM_FLAG_USER != 0 {
        user_access = true
    };
    if flags & VM_FLAG_GLOBAL != 0 {
        global = true
    };
    if flags & VM_FLAG_EXEC != 0 {
        no_execute = false
    };
    if flags & VM_FLAG_CACHE_DISABLE != 0 {
        cache_disable = true
    };
    if flags & VM_FLAG_WRITE_THROUGH != 0 {
        write_through = true
    };
    get_flags(true, writable, user_access, write_through, cache_disable, false, false, false, global, no_execute) as usize
}

pub struct VmaNode {
    pub start: usize,
    pub size: usize,
    pub flags: usize,
    pub prev: Option<*mut VmaNode>,
    pub next: Option<*mut VmaNode>,
}

pub fn allocate_node() -> *mut VmaNode {
    let layout = Layout::new::<VmaNode>();
    unsafe {
        let ptr = alloc(layout);
        if ptr.is_null() {
            panic!("kmalloc failed to allocate VmaNode");
        }
        ptr as *mut VmaNode
    }
}

pub struct VirtMemManager {
    head: Option<*mut VmaNode>,
    pager: &'static TicketLock<Pager>,
    allocator: &'static PCAllocator,
}

unsafe impl Send for VirtMemManager {}
unsafe impl Sync for VirtMemManager {}

fn align_up(addr: usize) -> usize { (addr + 0xFFF) & !0xFFF }

impl VirtMemManager {
    pub const fn new(pager: &'static TicketLock<Pager>, allocator: &'static PCAllocator) -> Self { Self { head: None, pager, allocator } }

    pub fn mmap(&mut self, mut size: usize, flags: usize) -> Option<usize> {
        let mask = if flags & VM_FLAG_HUGE != 0 { HUGE_PAGE_SIZE - 1 } else { NORMAL_PAGE_SIZE - 1 };

        size = (size + mask) & !mask;

        let mut base = VM_BASE_ADDR;
        let mut gap_start: Option<usize> = None;
        let mut prev_ptr = None;
        let mut current_ptr = self.head;

        unsafe {
            if let Some(head_ptr) = current_ptr {
                let curr_node = &*head_ptr;
                if curr_node.start > base && (curr_node.start - base) >= size {
                    gap_start = Some(base);
                }
            } else {
                gap_start = Some(base);
            }

            if gap_start.is_none() {
                while let Some(curr_ptr) = current_ptr {
                    let curr_node = &*curr_ptr;
                    base = (curr_node.start + curr_node.size + mask) & !mask;

                    let next_ptr = curr_node.next;

                    if let Some(n_ptr) = next_ptr {
                        let next_node = &*n_ptr;
                        if next_node.start > base && (next_node.start - base) >= size {
                            gap_start = Some(base);
                            prev_ptr = Some(curr_ptr);
                            current_ptr = next_ptr;
                            break;
                        }
                    }

                    prev_ptr = Some(curr_ptr);
                    current_ptr = next_ptr;
                }
            }

            if gap_start.is_none() {
                if let Some(last_ptr) = prev_ptr {
                    let last_node = &*last_ptr;
                    base = (last_node.start + last_node.size + mask) & !mask;
                    if VM_MAX_ALLOWED - base >= size {
                        gap_start = Some(base);
                    }
                }
            }
        }

        if let Some(addr) = gap_start {
            unsafe {
                let new_node_ptr = allocate_node();
                *new_node_ptr = VmaNode { start: addr, size, flags, prev: prev_ptr, next: current_ptr };

                if let Some(prev) = prev_ptr {
                    (*prev).next = Some(new_node_ptr);
                } else {
                    self.head = Some(new_node_ptr);
                }

                if let Some(next) = current_ptr {
                    (*next).prev = Some(new_node_ptr);
                }
            }
            return Some(addr);
        }
        None
    }

    pub fn munmap(&mut self, start_addr: usize, mut size: usize) -> Result<(), &'static str> {
        size = align_up(size);

        let mut current_ptr: Option<*mut VmaNode> = self.head;
        let mut target_vma_ptr: Option<*mut VmaNode> = None;

        unsafe {
            while let Some(curr) = current_ptr {
                let node = &mut *curr;

                if node.start == start_addr {
                    if node.size != size {
                        return Err("Size does not match VMA region");
                    }

                    // Detach from the list
                    if let Some(prev) = node.prev {
                        (*prev).next = node.next;
                    } else {
                        self.head = node.next;
                    }

                    if let Some(next) = node.next {
                        (*next).prev = node.prev;
                    }

                    target_vma_ptr = Some(curr);
                    break;
                }
                current_ptr = node.next;
            }
        }

        let target_vma = match target_vma_ptr {
            Some(ptr) => unsafe { &*ptr },
            None => return Err("Invalid address or VMA not found"),
        };

        let is_huge = target_vma.flags & VM_FLAG_HUGE != 0;
        let step_size = if is_huge { HUGE_PAGE_SIZE } else { NORMAL_PAGE_SIZE };
        let block_size = if is_huge { BlockSize::Huge } else { BlockSize::Normal };

        let mut current_page = target_vma.start;
        let end_page = target_vma.start + target_vma.size;

        while current_page < end_page {
            let virt = VirtAddress(current_page as u64);
            let mut phys_to_free = None;

            {
                let mut pagerlock = self.pager.lock();
                if let Some(phys_addr) = pagerlock.translate(virt, *HHDMOFFSET as u64) {
                    phys_to_free = Some(phys_addr as usize);
                    pagerlock.unmap_page(virt, *HHDMOFFSET as u64, block_size);
                }
            }

            shootdown(current_page, size);

            if let Some(phys_addr) = phys_to_free {
                self.allocator.free(phys_addr, block_size);
            }

            current_page += step_size;
        }

        unsafe {
            dealloc(target_vma_ptr.unwrap() as *mut u8, Layout::new::<VmaNode>());
        }

        Ok(())
    }

    pub fn mprotect(&mut self, start_addr: usize, mut size: usize, new_flags: usize) -> Result<(), &'static str> {
        size = align_up(size);

        let mut current_ptr: Option<*mut VmaNode> = self.head;
        let mut target_vma_ptr: Option<*mut VmaNode> = None;

        unsafe {
            while let Some(curr) = current_ptr {
                let node = &mut *curr;

                if node.start == start_addr {
                    if node.size != size {
                        return Err("Size does not match VMA region exactly");
                    }
                    node.flags = new_flags;
                    target_vma_ptr = Some(curr);
                    break;
                }
                current_ptr = node.next;
            }
        }

        let target_vma = match target_vma_ptr {
            Some(ptr) => unsafe { &*ptr },
            None => return Err("Invalid address or VMA not found"),
        };

        let is_huge = target_vma.flags & VM_FLAG_HUGE != 0;
        let step_size = if is_huge { HUGE_PAGE_SIZE } else { NORMAL_PAGE_SIZE };
        let block_size = if is_huge { BlockSize::Huge } else { BlockSize::Normal };

        let mut current_page = target_vma.start;

        while current_page < (target_vma.start + target_vma.size) {
            let virt = VirtAddress(current_page as u64);
            let hwflags = convert_vm_flags(new_flags) as u64;
            {
                self.pager.lock().change_flags(virt, hwflags, *HHDMOFFSET as u64, block_size);
            }
            flush_tlb(current_page as u64);
            current_page += step_size;
        }
        Ok(())
    }

    pub fn handle_page_fault(&self, addr: usize, error_code: usize) -> bool {
        let mut target_vma_ptr = None;
        let mut current_ptr = self.head;

        unsafe {
            while let Some(curr) = current_ptr {
                let node = &*curr;
                if addr >= node.start && addr < (node.start + node.size) {
                    target_vma_ptr = Some(curr);
                    break;
                }
                current_ptr = node.next;
            }
        }

        let target_vma = match target_vma_ptr {
            Some(ptr) => unsafe { &*ptr },
            None => return false,
        };

        let is_write = (error_code & (1 << 1)) != 0;
        let vma_allows_write = (target_vma.flags & VM_FLAG_WRITE) != 0;

        if is_write && !vma_allows_write {
            return false; // tried writing to a read only vma which is very illegal and a real fault
        }

        let is_huge = target_vma.flags & VM_FLAG_HUGE != 0;
        let block_size = if is_huge { BlockSize::Huge } else { BlockSize::Normal };
        let mask = if is_huge { HUGE_PAGE_SIZE - 1 } else { NORMAL_PAGE_SIZE - 1 };

        let fault_page = addr & !mask;
        let virt = VirtAddress(fault_page as u64);

        let phys_frame = self.allocator.alloc(block_size) as u64;

        let hw_flags = convert_vm_flags(target_vma.flags) as u64;
        let mut pagerlock = self.pager.lock();
        pagerlock
            .map_page(virt, phys_frame, hw_flags, *HHDMOFFSET as u64, block_size)
            .expect("FATAL: Pager failed to map memory during Page Fault!");
        drop(pagerlock);

        flush_tlb(addr as u64);

        true
    }
}
