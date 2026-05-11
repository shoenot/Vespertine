#![allow(unreachable_code)]
#![no_std]
#![no_main]
mod arch;
mod boot;
mod drivers;
mod helpers;
mod kernel;
mod panic;
mod tests;

extern crate alloc;

pub use arch::x86_64::{
    IO_APIC,
    LOCAL_APIC,
};
use arch::x86_64::{
    init_apic,
    init_interrupts,
    cpu::fpu::{
        init_default_fpu_cxt,
        init_cr4,
    },
};
pub use boot::*;
use kernel::{
    sync::TicketLock,
    memory::{
        heap::KernelAllocator,
        paging::*,
        pmm::*,
        vmm::*,
    },
    thread::schedule::*,
    time,
    time::*,
};
use panic::hcf;
use tests::memory_tests::*;

use crate::kernel::sync::Mutex;

#[global_allocator]
pub static KERNEL_ALLOCATOR: KernelAllocator = KernelAllocator::new();

static ALLOCATOR: TicketLock<Allocator> = TicketLock::new(Allocator::new());
static PAGER: TicketLock<Pager> = TicketLock::new(Pager::new(&ALLOCATOR));
static GLOBAL_VMM: TicketLock<VirtMemManager> = TicketLock::new(VirtMemManager::new(&PAGER, &ALLOCATOR));

static SHARED_COUNTER: Mutex<usize> = Mutex::new(0);

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

    klog!("RUNNING MEMORY TESTS... ");

    test_kmalloc(false);
    test_vmalloc(false);
    test_collections(false);

    klogln!("TESTS COMPLETE!");

    init_apic();

    unsafe { init_cr4() };

    time::init();

    init_default_fpu_cxt();

    let tt1 = test_thread_1 as *const ();
    let tt2 = test_thread_2 as *const ();

    SCHEDULER.lock().init();

    SCHEDULER.lock().spawn(tt1 as usize).unwrap();
    SCHEDULER.lock().spawn(tt2 as usize).unwrap();

    arm_sleep_ns(10_000_000);

    SCHEDULER.lock().schedule();

    hcf();
}

fn test_thread_1() -> ! {
    loop {
        klogln!("T1: attempting to lock...");

        {
            let mut guard = SHARED_COUNTER.lock();
            klogln!("T1: lock acquired! counter is: {}", *guard);

            *guard += 1;

            sleep(10_000_000);

            klogln!("T1: Releasing lock...");
        }

        SCHEDULER.lock().schedule();

        sleep(10_000_000);
    }
}

fn test_thread_2() -> ! {
    loop {
        klogln!("T2: attempting to lock...");

        {
            let mut guard = SHARED_COUNTER.lock();
            klogln!("T2: lock acquired! counter is: {}", *guard);

            *guard += 1;

            sleep(10_000_000);

            klogln!("T2: Releasing lock...");
        }

        SCHEDULER.lock().schedule();

        sleep(10_000_000);
    }
}
