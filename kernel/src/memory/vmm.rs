#![allow(dead_code)]

use core::alloc::GlobalAlloc;
use core::ptr;

use alloc::alloc::{
    alloc,
    dealloc,
    Layout,
};
use alloc::sync::Arc;

// TODO: Optimize VMM tlb shootdowns. make it loop and unmap all the pages first and *then* fire the
// ipis.
use super::paging::*;
use super::pmm::*;
use crate::arch::x86_64::interrupts::shootdown::shootdown;
use crate::core::object::invoke::InvocationError;
use crate::core::sync::TicketLock;
use crate::memory::vmo::PagedBackingStore;
use crate::memory::{PCAllocator, GLOBAL_PMM};

pub static VM_FLAG_WRITE: usize = 1 << 0;
pub static VM_FLAG_EXEC: usize = 1 << 1;
pub static VM_FLAG_USER: usize = 1 << 2;
pub static VM_FLAG_HUGE: usize = 1 << 3;
pub static VM_FLAG_GLOBAL: usize = 1 << 4;
pub static VM_FLAG_CACHE_DISABLE: usize = 1 << 5;
pub static VM_FLAG_WRITE_THROUGH: usize = 1 << 6;

static VM_BASE_ADDR: usize = 0x4000_0000;
static VM_MAX_ALLOWED: usize = 0x0000_7FFF_FFFF_F000;
const BATCH_SIZE: usize = 64;

#[derive(Debug)]
pub enum FaultError {
    Invocation(InvocationError),
    InvalidAddress,
    AccessDenied,
    MappingFailed,
}

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
    pub backing_vmo: Option<Arc<dyn PagedBackingStore>>,
    pub vmo_offset: usize,
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

#[derive(Debug)]
pub struct VirtMemManager {
    head: Option<*mut VmaNode>,
    pager: TicketLock<Pager>,
    allocator: &'static PCAllocator,
}

unsafe impl Send for VirtMemManager {}
unsafe impl Sync for VirtMemManager {}

pub fn align_up(addr: usize) -> usize { (addr + 0xFFF) & !0xFFF }

impl VirtMemManager {
    pub fn new(allocator: &'static PCAllocator) -> Self { 
        let mut pager = Pager::new(allocator);
        pager.init_process_pager().expect("Failed to initialize process pager");

        Self { head: None, pager: TicketLock::new(pager), allocator } 
    }

    pub fn get_pml4_addr(&self) -> usize {
        self.pager.lock().get_l4_addr() as usize
    }

    // temp for now
    pub fn mmap(&mut self, size: usize, flags: usize) -> Option<usize> {
        let node_ptr = allocate_node();
        self.mmap_internal(size, flags, None, 0, node_ptr)
    }

    pub fn mmap_internal(&mut self, mut size: usize, flags: usize, backing_vmo: Option<Arc<dyn PagedBackingStore>>, vmo_offset: usize, node_ptr: *mut VmaNode) -> Option<usize> {
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
                ptr::write(node_ptr, VmaNode { 
                    start: addr, size, flags, 
                    prev: prev_ptr, next: current_ptr,
                    backing_vmo, vmo_offset,
                });

                if let Some(prev) = prev_ptr {
                    (*prev).next = Some(node_ptr);
                } else {
                    self.head = Some(node_ptr);
                }

                if let Some(next) = current_ptr {
                    (*next).prev = Some(node_ptr);
                }
            }
            return Some(addr);
        }
        unsafe { dealloc(node_ptr as *mut u8, Layout::new::<VmaNode>()); }
        None
    }

    pub fn mmap_vmo(&mut self, size: usize, flags: usize, backing_vmo: Arc<dyn PagedBackingStore>) -> Option<usize> {
        let node_ptr = allocate_node();
        self.mmap_internal(size, flags, Some(backing_vmo), 0, node_ptr)
    }

    pub fn mmap_vmo_at(&mut self, start_addr: usize, mut size: usize, flags: usize, backing_vmo: Arc<dyn PagedBackingStore>) -> Option<usize> {
        let mask = NORMAL_PAGE_SIZE - 1;
        size = (size + mask) & !mask;

        let mut prev_ptr = None;
        let mut current_ptr = self.head;

        unsafe {
            // find spot where start_addr fits
            while let Some(curr) = current_ptr {
                if (*curr).start > start_addr {
                    break;
                }
                prev_ptr = Some(curr);
                current_ptr = (*curr).next;
            }

            // check for overlaps with prv mapping
            if let Some(prev) = prev_ptr {
                    if (*prev).start + (*prev).size > start_addr { return None };
            }

            // check for overlaps with next mapping
            if let Some(next) = current_ptr {
                if start_addr + size > (*next).start { return None };
            }

            let node_ptr = allocate_node();
            ptr::write(node_ptr, VmaNode {
                start: start_addr,
                size,
                flags,
                prev: prev_ptr,
                next: current_ptr,
                backing_vmo: Some(backing_vmo),
                vmo_offset: 0,
            });

            if let Some(prev) = prev_ptr {
                (*prev).next = Some(node_ptr);
            } else {
                self.head = Some(node_ptr);
            }

            if let Some(next) = current_ptr {
                (*next).prev = Some(node_ptr);
            }

            Some(start_addr)
        }
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

        let mut phys_batch = [0usize; BATCH_SIZE];

        while current_page < end_page {
            let mut batch_count = 0;
            let batch_start = current_page;

            {
                let mut pagerlock = self.pager.lock();

                while current_page < end_page && batch_count < BATCH_SIZE {
                    let virt = VirtAddress(current_page as u64);

                    if let Some(phys_addr) = pagerlock.translate(virt, *HHDMOFFSET as u64) {
                        phys_batch[batch_count] = phys_addr as usize;
                        batch_count += 1;
                        pagerlock.unmap_page(virt, *HHDMOFFSET as u64, block_size);
                    }
                    current_page += step_size;
                }
            }

            // fire ipis by batches because doing it for every page is bad for performance
            if batch_count > 0 {
                let batch_size_bytes = current_page - batch_start;
                shootdown(batch_start, batch_size_bytes);
                for i in 0..batch_count {
                    self.allocator.free(phys_batch[i],block_size);
                }
            }
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

    pub fn handle_page_fault(&self, addr: usize, error_code: usize) -> Result<(), FaultError> {
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

        let target_vma = target_vma_ptr
            .map(|ptr| unsafe { &*ptr })
            .ok_or(FaultError::InvalidAddress)?;  // if vma not found that means segfault

        let is_write = (error_code & (1 << 1)) != 0;
        let vma_allows_write = (target_vma.flags & VM_FLAG_WRITE) != 0;

        if is_write && !vma_allows_write {
            return Err(FaultError::AccessDenied); // tried writing to a read only vma which is very illegal and a real fault
        }

        let is_huge = target_vma.flags & VM_FLAG_HUGE != 0;
        let block_size = if is_huge { BlockSize::Huge } else { BlockSize::Normal };
        let mask = if is_huge { HUGE_PAGE_SIZE - 1 } else { NORMAL_PAGE_SIZE - 1 };

        let fault_page = addr & !mask;
        let virt = VirtAddress(fault_page as u64);
        let offset_in_vma = fault_page - target_vma.start;
        let vmo_offset = offset_in_vma + target_vma.vmo_offset;

        let phys_frame = if let Some(ref obj) = target_vma.backing_vmo {
            // if vmo already has the page then use it 
            match obj.request_page(vmo_offset) {
                Ok(addr) => addr,
                Err(_) => return Err(FaultError::MappingFailed),
            }
        } else {
            // else get it from the allocator
            self.allocator.alloc(block_size) as usize
        };

        let hw_flags = convert_vm_flags(target_vma.flags) as u64;
        let mut pagerlock = self.pager.lock();
        pagerlock
            .map_page(virt, phys_frame as u64, hw_flags, *HHDMOFFSET as u64, block_size)
            .expect("FATAL: Pager failed to map memory during Page Fault!");
        drop(pagerlock);

        flush_tlb(addr as u64);
        Ok(())
    }

    pub fn teardown(&mut self) {
        unsafe { 
            while let Some(node_ptr) = self.head {
                let start = (*node_ptr).start;
                let size = (*node_ptr).size;
                let _ = self.munmap(start, size);
            }

            let pagerlock = self.pager.lock();
            let pml4_phys = pagerlock.get_l4_addr();

            let pml4 = &mut *((pml4_phys + *HHDMOFFSET as u64) as *mut PageTable);
            for idx in 0..256 {
                let entry = &mut pml4.entries[idx];
                if entry.is_present() {
                    let l3_phys = entry.get_addr();

                    let l3 = &mut *((l3_phys + *HHDMOFFSET as u64) as *mut PageTable);
                    for l3_idx in 0..512 {
                        let l3_entry = &mut l3.entries[l3_idx];
                        if l3_entry.is_present() {
                            let l2_phys = entry.get_addr();

                            let l2 = &mut *((l2_phys + *HHDMOFFSET as u64) as *mut PageTable);
                            for l2_idx in 0..512 {
                                let l2_entry = &mut l2.entries[l2_idx];
                                if l2_entry.is_present() && l2_entry.is_huge() {
                                    let l1_phys = l2_entry.get_addr();
                                    GLOBAL_PMM.lock().free(l1_phys as usize, BlockSize::Normal);
                                }
                            }
                            GLOBAL_PMM.lock().free(l2_phys as usize, BlockSize::Normal);
                        }
                    }
                    GLOBAL_PMM.lock().free(l3_phys as usize, BlockSize::Normal);
                }
            }
            GLOBAL_PMM.lock().free(pml4_phys as usize, BlockSize::Normal);
        }
    }

    pub fn get_total_allocated_size(&self) -> usize {
        let mut total = 0;
        let mut current_ptr = self.head;
        unsafe { 
            while let Some(curr) = current_ptr {
                total += (*curr).size;
                current_ptr = (*curr).next;
            }
        }
        total
    }
}
