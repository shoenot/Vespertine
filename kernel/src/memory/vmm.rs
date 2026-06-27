#![allow(dead_code)]

use alloc::alloc::{
    Layout,
    alloc,
    dealloc,
};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr::{
    self,
    drop_in_place,
};

// TODO: Optimize VMM tlb shootdowns. make it loop and unmap all the pages first and *then* fire the
// ipis.
use super::paging::*;
use super::pmm::*;
use crate::arch::x86_64::interrupts::shootdown::shootdown;
use crate::core::object::invoke::InvocationError;
use crate::core::sync::TicketLock;
use crate::memory::vmo::PagedBackingStore;
use crate::memory::{
    GLOBAL_PMM,
    PCAllocator,
};
use crate::storage::fs::VfsNode;

pub static VM_FLAG_WRITE: usize = 1 << 0;
pub static VM_FLAG_EXEC: usize = 1 << 1;
pub static VM_FLAG_USER: usize = 1 << 2;
pub static VM_FLAG_HUGE: usize = 1 << 3;
pub static VM_FLAG_GLOBAL: usize = 1 << 4;
pub static VM_FLAG_CACHE_DISABLE: usize = 1 << 5;
pub static VM_FLAG_WRITE_THROUGH: usize = 1 << 6;
pub static VM_FLAG_NO_ACCESS: usize = 1 << 7;

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
    pub backing_nodes: Vec<Arc<dyn VfsNode>>,
}

unsafe impl Send for VirtMemManager {}
unsafe impl Sync for VirtMemManager {}

pub fn align_up(addr: usize) -> usize { (addr + 0xFFF) & !0xFFF }

impl VirtMemManager {
    pub fn new(allocator: &'static PCAllocator) -> Self {
        let mut pager = Pager::new(allocator);
        pager.init_process_pager().expect("Failed to initialize process pager");

        Self { head: None, pager: TicketLock::new(pager), allocator, backing_nodes: Vec::new() }
    }

    pub fn get_pml4_addr(&self) -> usize { self.pager.lock().get_l4_addr() as usize }

    // temp for now
    pub fn mmap(&mut self, size: usize, flags: usize) -> Option<usize> {
        let node_ptr = allocate_node();
        self.mmap_internal(size, flags, None, 0, node_ptr)
    }

    pub fn mmap_internal(
        &mut self, mut size: usize, flags: usize, backing_vmo: Option<Arc<dyn PagedBackingStore>>, vmo_offset: usize,
        node_ptr: *mut VmaNode,
    ) -> Option<usize> {
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

                    // Clamp base to ensure low-memory segments do not pull the
                    // allocator search region below VM_BASE_ADDR
                    if base < VM_BASE_ADDR {
                        base = VM_BASE_ADDR;
                    }

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

                    // Clamp fallback allocation base to stay out of the low-memory zero page zone
                    if base < VM_BASE_ADDR {
                        base = VM_BASE_ADDR;
                    }

                    if VM_MAX_ALLOWED - base >= size {
                        gap_start = Some(base);
                    }
                }
            }
        }

        if let Some(addr) = gap_start {
            unsafe {
                ptr::write(node_ptr, VmaNode { start: addr, size, flags, prev: prev_ptr, next: current_ptr, backing_vmo, vmo_offset });

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
        unsafe {
            dealloc(node_ptr as *mut u8, Layout::new::<VmaNode>());
        }
        None
    }

    pub fn mmap_vmo(&mut self, size: usize, flags: usize, backing_vmo: Arc<dyn PagedBackingStore>) -> Option<usize> {
        if let Some(node) = backing_vmo.get_node() {
            self.backing_nodes.push(node);
        }
        let node_ptr = allocate_node();
        self.mmap_internal(size, flags, Some(backing_vmo), 0, node_ptr)
    }

    pub fn mmap_vmo_at(
        &mut self, mut start_addr: usize, mut size: usize, flags: usize, backing_vmo: Arc<dyn PagedBackingStore>, mut vmo_offset: usize,
    ) -> Option<usize> {
        if let Some(node) = backing_vmo.get_node() {
            self.backing_nodes.push(node);
        }
        // Calculate page misalignment shift
        let page_offset = start_addr & (NORMAL_PAGE_SIZE - 1);

        // Align boundaries down, stretch size up to compensate
        if page_offset > 0 {
            start_addr -= page_offset;
            vmo_offset = vmo_offset.saturating_sub(page_offset);
            size += page_offset;
        }

        let mask = NORMAL_PAGE_SIZE - 1;
        size = (size + mask) & !mask;

        let mut prev_ptr = None;
        let mut current_ptr = self.head;

        unsafe {
            // Find the spot where start_addr fits sequentially
            while let Some(curr) = current_ptr {
                if (*curr).start > start_addr {
                    break;
                }
                prev_ptr = Some(curr);
                current_ptr = (*curr).next;
            }

            // Handle overlaps with an existing tracking block (e.g., ld.so placeholder reservations)
            if let Some(prev) = prev_ptr {
                let prev_end = (*prev).start + (*prev).size;
                if prev_end > start_addr {
                    if start_addr >= (*prev).start && start_addr + size <= prev_end {
                        let old_start = (*prev).start;
                        let old_size = (*prev).size;
                        let old_flags = (*prev).flags;
                        let old_vmo = (*prev).backing_vmo.clone();
                        let old_vmo_off = (*prev).vmo_offset;
                        let old_next = (*prev).next;

                        if start_addr == old_start && size == old_size {
                            let node_ptr = allocate_node();
                            ptr::write(
                                node_ptr,
                                VmaNode {
                                    start: start_addr,
                                    size,
                                    flags,
                                    prev: (*prev).prev,
                                    next: (*prev).next,
                                    backing_vmo: Some(backing_vmo),
                                    vmo_offset,
                                },
                            );

                            if let Some(p) = (*prev).prev {
                                (*p).next = Some(node_ptr);
                            } else {
                                self.head = Some(node_ptr);
                            }
                            if let Some(n) = (*prev).next {
                                (*n).prev = Some(node_ptr);
                            }
                            drop_in_place(prev);
                            dealloc(prev as *mut u8, Layout::new::<VmaNode>());
                            return Some(start_addr + page_offset);
                        } else if start_addr == old_start {
                            (*prev).start += size;
                            (*prev).size -= size;
                            (*prev).vmo_offset += size;

                            let node_ptr = allocate_node();
                            ptr::write(
                                node_ptr,
                                VmaNode {
                                    start: start_addr,
                                    size,
                                    flags,
                                    prev: (*prev).prev,
                                    next: Some(prev),
                                    backing_vmo: Some(backing_vmo),
                                    vmo_offset,
                                },
                            );

                            if let Some(p) = (*prev).prev {
                                (*p).next = Some(node_ptr);
                            } else {
                                self.head = Some(node_ptr);
                            }
                            (*prev).prev = Some(node_ptr);
                            return Some(start_addr + page_offset);
                        } else if start_addr + size == prev_end {
                            (*prev).size -= size;

                            let node_ptr = allocate_node();
                            ptr::write(
                                node_ptr,
                                VmaNode {
                                    start: start_addr,
                                    size,
                                    flags,
                                    prev: Some(prev),
                                    next: (*prev).next,
                                    backing_vmo: Some(backing_vmo),
                                    vmo_offset,
                                },
                            );

                            if let Some(n) = (*prev).next {
                                (*n).prev = Some(node_ptr);
                            }
                            (*prev).next = Some(node_ptr);
                            return Some(start_addr + page_offset);
                        } else {
                            (*prev).size = start_addr - old_start;
                            let middle_node = allocate_node();
                            let right_node = allocate_node();

                            ptr::write(
                                right_node,
                                VmaNode {
                                    start: start_addr + size,
                                    size: prev_end - (start_addr + size),
                                    flags: old_flags,
                                    prev: Some(middle_node),
                                    next: old_next,
                                    backing_vmo: old_vmo,
                                    vmo_offset: old_vmo_off + (start_addr + size - old_start),
                                },
                            );

                            ptr::write(
                                middle_node,
                                VmaNode {
                                    start: start_addr,
                                    size,
                                    flags,
                                    prev: Some(prev),
                                    next: Some(right_node),
                                    backing_vmo: Some(backing_vmo),
                                    vmo_offset,
                                },
                            );

                            if let Some(n) = old_next {
                                (*n).prev = Some(right_node);
                            }
                            (*prev).next = Some(middle_node);
                            return Some(start_addr + page_offset);
                        }
                    } else {
                        return None;
                    }
                }
            }

            if let Some(next) = current_ptr {
                if start_addr + size > (*next).start {
                    return None;
                };
            }

            let node_ptr = allocate_node();
            ptr::write(
                node_ptr,
                VmaNode { start: start_addr, size, flags, prev: prev_ptr, next: current_ptr, backing_vmo: Some(backing_vmo), vmo_offset },
            );

            if let Some(prev) = prev_ptr {
                (*prev).next = Some(node_ptr);
            } else {
                self.head = Some(node_ptr);
            }
            if let Some(next) = current_ptr {
                (*next).prev = Some(node_ptr);
            }

            // Always return the original requested unaligned entry pointer to the loader context
            Some(start_addr + page_offset)
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
        let mut offset_batch = [0usize; BATCH_SIZE];

        while current_page < end_page {
            let mut batch_count = 0;
            let batch_start = current_page;

            {
                let mut pagerlock = self.pager.lock();

                while current_page < end_page && batch_count < BATCH_SIZE {
                    let virt = VirtAddress(current_page as u64);

                    if let Some(phys_addr) = pagerlock.translate(virt, *HHDMOFFSET as u64) {
                        phys_batch[batch_count] = phys_addr as usize;
                        offset_batch[batch_count] = current_page - target_vma.start; // Save VMA offset
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

                if let Some(ref vmo) = target_vma.backing_vmo {
                    for i in 0..batch_count {
                        let phys_addr = phys_batch[i];
                        let vmo_offset = target_vma.vmo_offset + offset_batch[i];

                        // if the vmo doesn't own this exact physical page, it's a private CoW copy
                        if vmo.peek_page(vmo_offset) != Some(phys_addr) {
                            self.allocator.free(phys_addr, block_size);
                        }
                    }
                } else {
                    // anonymous vma: we own everything, free it all
                    for i in 0..batch_count {
                        self.allocator.free(phys_batch[i], block_size);
                    }
                }
            }
        }

        unsafe {
            drop_in_place(target_vma_ptr.unwrap());
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

                if start_addr >= node.start && start_addr + size <= node.start + node.size {
                    target_vma_ptr = Some(curr);
                    break;
                }
                current_ptr = node.next;
            }
        }

        let target_vma = match target_vma_ptr {
            Some(ptr) => unsafe { &mut *ptr },
            None => return Err("Invalid address or VMA not found"),
        };

        let is_huge = target_vma.flags & VM_FLAG_HUGE != 0;
        let step_size = if is_huge { HUGE_PAGE_SIZE } else { NORMAL_PAGE_SIZE };
        let block_size = if is_huge { BlockSize::Huge } else { BlockSize::Normal };

        unsafe {
            let old_start = target_vma.start;
            let old_size = target_vma.size;
            let old_flags = target_vma.flags;
            let old_vmo = target_vma.backing_vmo.clone();
            let old_vmo_off = target_vma.vmo_offset;
            let old_next = target_vma.next;

            if start_addr == old_start && size == old_size {
                target_vma.flags = new_flags;
            } else if start_addr == old_start {
                target_vma.start += size;
                target_vma.size -= size;
                target_vma.vmo_offset += size;

                let node_ptr = allocate_node();
                ptr::write(
                    node_ptr,
                    VmaNode {
                        start: start_addr,
                        size,
                        flags: new_flags,
                        prev: target_vma.prev,
                        next: Some(target_vma_ptr.unwrap()),
                        backing_vmo: old_vmo,
                        vmo_offset: old_vmo_off,
                    },
                );

                if let Some(p) = target_vma.prev {
                    (*p).next = Some(node_ptr);
                } else {
                    self.head = Some(node_ptr);
                }
                target_vma.prev = Some(node_ptr);
            } else if start_addr + size == old_start + old_size {
                target_vma.size -= size;

                let node_ptr = allocate_node();
                ptr::write(
                    node_ptr,
                    VmaNode {
                        start: start_addr,
                        size,
                        flags: new_flags,
                        prev: Some(target_vma_ptr.unwrap()),
                        next: target_vma.next,
                        backing_vmo: old_vmo,
                        vmo_offset: old_vmo_off + (start_addr - old_start),
                    },
                );

                if let Some(n) = target_vma.next {
                    (*n).prev = Some(node_ptr);
                }
                target_vma.next = Some(node_ptr);
            } else {
                target_vma.size = start_addr - old_start;
                let middle_node = allocate_node();
                let right_node = allocate_node();

                ptr::write(
                    right_node,
                    VmaNode {
                        start: start_addr + size,
                        size: old_start + old_size - (start_addr + size),
                        flags: old_flags,
                        prev: Some(middle_node),
                        next: old_next,
                        backing_vmo: old_vmo.clone(),
                        vmo_offset: old_vmo_off + (start_addr + size - old_start),
                    },
                );

                ptr::write(
                    middle_node,
                    VmaNode {
                        start: start_addr,
                        size,
                        flags: new_flags,
                        prev: Some(target_vma_ptr.unwrap()),
                        next: Some(right_node),
                        backing_vmo: old_vmo,
                        vmo_offset: old_vmo_off + (start_addr - old_start),
                    },
                );

                if let Some(n) = old_next {
                    (*n).prev = Some(right_node);
                }
                target_vma.next = Some(middle_node);
            }
        }

        let mut current_page = start_addr;

        // If we are making a file-backed VMA writable, we must unmap any existing
        // read-only entries to force a page fault and trigger Copy-on-Write.
        let needs_cow_reset =
            (new_flags & VM_FLAG_WRITE) != 0 && target_vma.backing_vmo.as_ref().map(|v| v.get_node().is_some()).unwrap_or(false);

        while current_page < (start_addr + size) {
            let virt = VirtAddress(current_page as u64);
            let hwflags = convert_vm_flags(new_flags) as u64;
            {
                let mut pager = self.pager.lock();
                if new_flags & VM_FLAG_NO_ACCESS != 0 || needs_cow_reset {
                    pager.unmap_page(virt, *HHDMOFFSET as u64, block_size);
                } else {
                    pager.change_flags(virt, hwflags, *HHDMOFFSET as u64, block_size);
                }
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

        let target_vma = target_vma_ptr.map(|ptr| unsafe { &*ptr }).ok_or(FaultError::InvalidAddress)?; // if vma not found that means segfault

        if target_vma.flags & VM_FLAG_NO_ACCESS != 0 {
            return Err(FaultError::AccessDenied);
        }

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
            let page = match obj.request_page(vmo_offset) {
                Ok(addr) => addr,
                Err(_) => return Err(FaultError::MappingFailed),
            };

            // If the VMA is writable and backed by a file (has a node),
            // we must provide a private copy to avoid contaminating the global cache.
            if vma_allows_write && obj.get_node().is_some() {
                let private_page = self.allocator.alloc(block_size) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping((page + *HHDMOFFSET) as *const u8, (private_page + *HHDMOFFSET) as *mut u8, mask + 1);
                }
                private_page
            } else {
                page
            }
        } else {
            // else get it from the allocator and ZERO it
            let new_frame = self.allocator.alloc(block_size) as usize;
            if new_frame != 0 {
                unsafe {
                    // mask + 1 correctly handles both NORMAL_PAGE_SIZE and HUGE_PAGE_SIZE
                    core::ptr::write_bytes((new_frame + *HHDMOFFSET) as *mut u8, 0, mask + 1);
                }
            }
            new_frame
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
                            let l2_phys = l3_entry.get_addr();

                            let l2 = &mut *((l2_phys + *HHDMOFFSET as u64) as *mut PageTable);
                            for l2_idx in 0..512 {
                                let l2_entry = &mut l2.entries[l2_idx];
                                if l2_entry.is_present() && !l2_entry.is_huge() {
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

    pub fn get_resident_size(&self) -> usize {
        let mut total = 0;
        let mut current_ptr = self.head;

        let mut pager = self.pager.lock();
        unsafe {
            while let Some(curr) = current_ptr {
                let node = &*curr;

                let is_huge = node.flags & VM_FLAG_HUGE != 0;
                let step_size = if is_huge { HUGE_PAGE_SIZE } else { NORMAL_PAGE_SIZE };

                let mut current_page = node.start;
                let end_page = node.start + node.size;

                while current_page < end_page {
                    let virt = VirtAddress(current_page as u64);
                    if pager.translate(virt, *HHDMOFFSET as u64).is_some() {
                        total += step_size;
                    }
                    current_page += step_size;
                }
                current_ptr = node.next;
            }
        }
        total
    }
}

impl Drop for VirtMemManager {
    fn drop(&mut self) {
        let mut current = self.head;
        while let Some(node) = current {
            unsafe {
                let next = (*node).next;
                drop_in_place(node);
                dealloc(node as *mut u8, Layout::new::<VmaNode>());
                current = next;
            }
        }
    }
}
