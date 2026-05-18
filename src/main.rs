#![no_std]
#![no_main]
mod arch;
mod boot;
mod demo;
mod drivers;
mod kernel;
mod memory;
mod panic;
mod tasks;
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

use crate::drivers::keyboard::init_keyboard_irq;
use crate::kernel::thread::dispatch::spawn_kernel_thread;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::time::datetime::epoch_to_datetime;
use crate::memory::GLOBAL_PMM;

pub static BOOTSTRAP_ALLOC: TicketLock<BumpAllocator> = TicketLock::new(BumpAllocator::new());

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    LOGGER.lock().init();

    memory::init();
    let bootstrap_page = GLOBAL_PMM.lock().alloc(BlockSize::Huge).unwrap() as usize;
    BOOTSTRAP_ALLOC.lock().init(bootstrap_page);

    arch::init();
    arch::init_fpu(true);
    arch::init_bootstrap_core();
    time::init();

    let cr3 = get_cr3();
    BSP_CR3.store(cr3, Ordering::Relaxed);

    init_smp();

    time::init_realtime();
    klogln!("Initialized Real Time Clock. Current time is: {}", epoch_to_datetime(time::get_realtime()));

    init_keyboard_irq();
    enable_interrupts();

    spawn_kernel_thread(tasks::initializer as *const () as usize, 0, ThreadPriority::MAXIMUM);

    terminate_thread!();
}
