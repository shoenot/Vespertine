mod bootalloc;
pub mod heap;
pub mod paging;
mod pmm;
mod vmm;

pub use bootalloc::*;
use heap::*;
use paging::*;
use pmm::*;
pub use pmm::{
    BlockSize,
    HHDMOFFSET,
};
use vmm::*;

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

pub static ALLOCATOR: TicketLock<Allocator> = TicketLock::new(Allocator::new());
pub static PAGER: TicketLock<Pager> = TicketLock::new(Pager::new(&ALLOCATOR));
pub static GLOBAL_VMM: RwLock<VirtMemManager> = RwLock::new(VirtMemManager::new(&PAGER, &ALLOCATOR));

pub fn init() {
    klogln!("INITIATING MEMORY MANAGERS... ");

    // Inititate PMM
    {
        let mut allocator = ALLOCATOR.lock();
        allocator.init();
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
