#![no_std]
#![no_main]
mod arch;
mod drivers;
mod kernel;
mod boot;
mod tests;
mod panic;

extern crate alloc;
pub use boot::*;

use panic::hcf;

use arch::x86_64::{init_interrupts, init_apic};
pub use arch::x86_64::{LOCAL_APIC, IO_APIC};

use kernel::lock::TicketLock;

use kernel::memory::pmm::*;
use kernel::memory::paging::*;
use kernel::memory::vmm::*;
use kernel::memory::heap::KernelAllocator;

use kernel::time;
use kernel::time::*;

use tests::memory_tests::*;

#[global_allocator]
pub static KERNEL_ALLOCATOR: KernelAllocator = KernelAllocator::new();

static ALLOCATOR: TicketLock<Allocator> = TicketLock::new(Allocator::new());
static PAGER: TicketLock<Pager> = TicketLock::new(Pager::new(&ALLOCATOR));
static GLOBAL_VMM: TicketLock<VirtMemManager> = TicketLock::new(VirtMemManager::new(&PAGER, &ALLOCATOR));

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    if !BASE_REVISION.is_supported() {
        hcf();
    }
    
    init_interrupts();

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
    
    klogln!("RUNNING MEMORY TESTS");
    
    test_kmalloc();
    test_vmalloc();
    test_collections();

    klogln!("TESTS COMPLETE!");

    init_apic();

    time::init();
    klogln!("Using timer: {:#?} with frequency: {:?}", *TIME_SOURCE.lock(), TIME_SRC_FQ);

    hcf();
}
