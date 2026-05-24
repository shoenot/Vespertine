mod bootalloc;
pub mod heap;
mod init_pmm;
pub mod magazine;
pub mod paging;
mod pmm;
pub mod vmm;
pub mod vmo;

pub use bootalloc::*;
use heap::*;
use paging::*;
use pmm::*;
pub use pmm::{
    BlockSize,
    HUGE_PAGE_SIZE,
    NORMAL_PAGE_SIZE,
};
use vmm::*;

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
    interrupts_enabled,
};
use crate::core::sync::{
    KernelOnceCell,
    TicketLock,
};
use crate::core::thread::get_current_process;
use crate::{
    HHDM_REQUEST,
    klog,
    klogln,
};

pub static HHDMOFFSET: KernelOnceCell<usize> = KernelOnceCell::new();

#[global_allocator]
pub static KERNEL_ALLOCATOR: KernelAllocator = KernelAllocator::new();

pub static GLOBAL_PMM: TicketLock<Allocator> = TicketLock::new(Allocator::new());
pub static ALLOCATOR: PCAllocator = PCAllocator {};
pub static PAGER: TicketLock<Pager> = TicketLock::new(Pager::new(&ALLOCATOR));

pub fn handle_page_fault(addr: usize, error_code: usize) -> Result<(), FaultError> { 
    if let Some(proc) = get_current_process() {
        proc.vmm.read().handle_page_fault(addr, error_code)
    } else {
        Err(FaultError::InvalidAddress)
    }
}

#[derive(Debug)]
pub struct PCAllocator {}

impl PCAllocator {
    pub fn alloc(&self, size: BlockSize) -> usize {
        match size {
            BlockSize::Huge => GLOBAL_PMM.lock().alloc(size).expect("Global PMM Exhausted"),
            BlockSize::Normal => {
                let int_state = interrupts_enabled();
                disable_interrupts();
                let ret = get_core_data().magazine.alloc();
                if int_state {
                    enable_interrupts();
                }
                ret
            }
        }
    }

    pub fn alloc_order(&self, order: usize) -> Option<usize> {
        GLOBAL_PMM.lock().alloc_order(order)
    }

    pub fn free(&self, addr: usize, size: BlockSize) {
        match size {
            BlockSize::Huge => {
                GLOBAL_PMM.lock().free(addr, size);
            }
            BlockSize::Normal => {
                let int_state = interrupts_enabled();
                disable_interrupts();
                get_core_data().magazine.free(addr);
                if int_state {
                    enable_interrupts();
                }
            }
        }
    }

    pub fn free_order(&self, addr: usize, order: usize) {
        GLOBAL_PMM.lock().free_order(addr, order)
    }
}

pub fn init() {
    klog!("INITIATING PMM... ");
    HHDMOFFSET.get_or_init(|| HHDM_REQUEST.response().expect("Failed to get HHDM offset from Limine").offset as usize);
    // Inititate PMM
    {
        let mut global_pmm = GLOBAL_PMM.lock();
        global_pmm.init();
    }
    klogln!("OK");
    // Inititate Pager
    {
        let mut pager = PAGER.lock();
        pager.init();
    }

    klogln!("SWITCHED CR3. PAGING HANDOVER COMPLETE.");

    // klog!("RUNNING MEMORY TESTS... ");
    //
    // memory_tests::test_kmalloc(false);
    // memory_tests::test_vmalloc(false);
    // memory_tests::test_collections(false);
    //
    // klogln!("TESTS COMPLETE!");
}
