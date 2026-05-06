use core::mem::{size_of, align_of};
use crate::kernel::lock::TicketLock;
use crate::kernel::memory::paging::*;
use crate::kernel::memory::pmm::*;
use crate::kernel::memory::bs_kmalloc::*;

static VM_FLAG_NONE: usize = 0;
static VM_FLAG_WRITE: usize = 1 << 0;
static VM_FLAG_EXEC: usize = 1 << 1;
static VM_FLAG_USER: usize = 1 << 2;

static VM_BASE_ADDR: usize = 0x4000_0000;

fn convert_vm_flags(flags: usize) -> usize {
    let mut writable = false;
    let mut no_execute = true;
    let mut user_access = false;
    if flags & VM_FLAG_WRITE != 0 { writable = true };
    if flags & VM_FLAG_USER  != 0 { user_access = true };
    if (flags & VM_FLAG_EXEC) != 0 { no_execute = false };
    get_flags(true, writable, user_access, false, false, false, false, false, false, no_execute) as usize
}

pub struct VmaNode {
    pub start: usize,
    pub size: usize,
    pub flags: usize,
    pub next: Option<*mut VmaNode>,
}

pub fn allocate_node() -> *mut VmaNode {
    let size = size_of::<VmaNode>();
    let align = align_of::<VmaNode>();
    let ptr = BOOTSTRAP_ALLOC.lock().alloc(size, align).expect("Bootstrap heap full");

    ptr as *mut VmaNode
}

pub struct VirtMemManager {
    head: Option<*mut VmaNode>,
    pager: &'static mut TicketLock<Pager>,
    allocator: &'static TicketLock<Allocator>,
}

fn align_up(addr: usize) -> usize {
    (addr + 0xFFF) & !0xFFF
}

impl VirtMemManager {
    pub const fn new(pager: &'static mut TicketLock<Pager>, allocator: &'static TicketLock<Allocator>) -> Self {
        Self { head: 0, pager, allocator }
    }

    pub fn mmap(&mut self, mut size: usize, flags: usize) -> Option<u64> {
        size = ((size + NORMAL_PAGE_SIZE - 1) / NORMAL_PAGE_SIZE) * NORMAL_PAGE_SIZE;
        
        let mut gap_start = 0;
        let mut next_addr = 0;
        let mut current_addr = 0;
        let mut base_addr = VM_BASE_ADDR;
        unsafe {
            if self.head.is_none() {
                gap_start = VM_BASE_ADDR;
            } else {
                let mut next_node = &
                cu = current_node.start;
                if current_node.start - VM_BASE_ADDR > size {
                    gap_start = VM_BASE_ADDR;
                } else {
                    while current_node.next.is_some() {
                        
                    }
                }
            }
        }
        None
    }

    pub fn munmap(&mut self, start: usize, size: usize) -> Result<(), &'statc str> {
        
    }

    pub fn mprotect(&mut self, start: usize, size: usize, new_flags: usize) -> Result<(), &'statc str> {

    }
}
