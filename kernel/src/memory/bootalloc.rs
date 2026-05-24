use crate::core::sync::TicketLock;
use crate::memory::pmm::HUGE_PAGE_SIZE;
use crate::memory::HHDMOFFSET;

pub static BOOTSTRAP_ALLOC: TicketLock<BumpAllocator> = TicketLock::new(BumpAllocator::new());

pub struct BumpAllocator {
    virt_base: usize,
    next: usize,
}

impl BumpAllocator {
    pub const fn new() -> Self { BumpAllocator { virt_base: 0, next: 0 } }

    pub fn init(&mut self, huge_page_phys: usize) {
        self.virt_base = huge_page_phys + *HHDMOFFSET;
        self.next = self.virt_base;
    }

    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        let alloc_base = (self.next + align - 1) & !(align - 1);
        let next = alloc_base.saturating_add(size);

        if next <= (self.virt_base + HUGE_PAGE_SIZE) {
            self.next = next;
            alloc_base as *mut u8
        } else {
            panic!("Bootstrap memory exhausted");
        }
    }
}
