#![no_std]
#![no_main]
mod arch;
mod boot;
mod demo;
mod drivers;
mod kernel;
mod memory;
mod panic;
mod tests;
mod util;

extern crate alloc;

use core::sync::atomic::Ordering;

use arch::x86_64::hcf;
use arch::{
    enable_interrupts,
    get_core_data,
};
use boot::smp::BSP_CR3;
pub use boot::*;
use drivers::logger::LOGGER;
use kernel::cpu::init_smp;
use kernel::sync::TicketLock;
use kernel::time;
use memory::paging::get_cr3;
use memory::{
    ALLOCATOR,
    BlockSize,
    BumpAllocator,
};

pub static BOOTSTRAP_ALLOC: TicketLock<BumpAllocator> = TicketLock::new(BumpAllocator::new());

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    LOGGER.lock().init();

    memory::init();
    let bootstrap_page = ALLOCATOR.lock().alloc(BlockSize::Huge).unwrap() as usize;
    BOOTSTRAP_ALLOC.lock().init(bootstrap_page);

    arch::init();
    arch::init_fpu(true);
    arch::init_bootstrap_core();
    time::init();

    let cr3 = get_cr3();
    BSP_CR3.store(cr3, Ordering::Relaxed);

    init_smp();

    enable_interrupts();
    get_core_data().scheduler.terminate();

    unreachable!()
}
