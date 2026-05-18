mod bootalloc;
pub mod heap;
pub mod paging;
mod pmm;
mod vmm;
mod init_pmm;
pub mod magazine;

pub use bootalloc::*;
use heap::*;
use paging::*;
use pmm::*;
pub use pmm::{
    BlockSize,
    HHDMOFFSET,
    NORMAL_PAGE_SIZE,
    HUGE_PAGE_SIZE,
};
use vmm::*;

use crate::arch::{disable_interrupts, enable_interrupts, get_core_data, interrupts_enabled};
use crate::kernel::sync::{
    RwLock,
    TicketLock,
};
use crate::{
    klog,
    klogln,
};

#[global_allocator]
pub static KERNEL_ALLOCATOR: KernelAllocator = KernelAllocator::new();

pub static GLOBAL_PMM: TicketLock<Allocator> = TicketLock::new(Allocator::new());
pub static ALLOCATOR: PCAllocator = PCAllocator {};
pub static PAGER: TicketLock<Pager> = TicketLock::new(Pager::new(&ALLOCATOR));
pub static GLOBAL_VMM: RwLock<VirtMemManager> = RwLock::new(VirtMemManager::new(&PAGER, &ALLOCATOR));

pub struct PCAllocator {}

impl PCAllocator {
    pub fn alloc(&self, size: BlockSize) -> usize {
        match size {
            BlockSize::Huge => {
                GLOBAL_PMM.lock().alloc(size).expect("Global PMM Exhausted")
            },
            BlockSize::Normal => {
                let int_state = interrupts_enabled();
                disable_interrupts();
                let ret = get_core_data().magazine.alloc();
                if int_state { enable_interrupts(); }
                ret
            }
        }
    }

    pub fn free(&self, addr: usize, size: BlockSize) {
        match size {
            BlockSize::Huge => {
                GLOBAL_PMM.lock().free(addr, size);
            },
            BlockSize::Normal => {
                let int_state = interrupts_enabled();
                disable_interrupts();
                get_core_data().magazine.free(addr);
                if int_state { enable_interrupts(); }
            }
        }
    }
}



pub fn init() {
    klogln!("INITIATING MEMORY MANAGERS... ");

    // Inititate PMM
    {
        let mut global_pmm = GLOBAL_PMM.lock();
        global_pmm.init();
    }

    // Inititate Pager
    {
        let mut pager = PAGER.lock();
        pager.init();
    }

    klogln!("SWITCHED CR3. PAGING HANDOVER COMPLETE.");

    klog!("RUNNING MEMORY TESTS... ");

    // memory_tests::test_kmalloc(false);
    // memory_tests::test_vmalloc(false);
    // memory_tests::test_collections(false);

    klogln!("TESTS COMPLETE!");
}
