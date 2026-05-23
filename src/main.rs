#![no_std]
#![no_main]
extern crate alloc;
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

use core::sync::atomic::Ordering;

use alloc::sync::Arc;
use arch::x86_64::hcf;
use arch::{
    enable_interrupts,
    get_core_data,
};
use boot::smp::BSP_CR3;
pub use boot::*;
use drivers::logger::LOGGER;
use kernel::cpu::init_smp;
use kernel::time;
use memory::paging::get_cr3;
use memory::{
    BOOTSTRAP_ALLOC,
    BlockSize,
};

use crate::arch::x86_64::cpu::core::{CPULocalData, init_timer_daemon};
use crate::drivers::keyboard::init_keyboard_irq;
use crate::kernel::object::handle::{AccessRights, HandleID};
use crate::kernel::object::models::directory::Directory;
use crate::kernel::object::vfs::ROOT_DIRECTORY;
use crate::kernel::object::models::process::{Process, ProcessControlBlock};
use crate::kernel::sync::KernelOnceCell;
use crate::kernel::thread::dispatch::spawn_kernel_thread;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::time::datetime::epoch_to_datetime;
use crate::memory::GLOBAL_PMM;

pub static KERNEL_PROCESS: KernelOnceCell<Process> = KernelOnceCell::new();

pub fn init_kernel_process() {
    KERNEL_PROCESS.get_or_init(|| 
        ProcessControlBlock::new(
            ROOT_DIRECTORY.get_or_init(|| Arc::new(Directory::new())).clone(),
            AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE | AccessRights::CREATE
        )
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    LOGGER.lock().init();

    memory::init();
    let bootstrap_page = GLOBAL_PMM.lock().alloc(BlockSize::Huge).unwrap() as usize;
    BOOTSTRAP_ALLOC.lock().init(bootstrap_page);

    arch::init();
    arch::init_fpu(true);

    arch::init_bootstrap_core();

    init_kernel_process();

    get_core_data().scheduler.init_threads(0);

    time::init();
    let data_ptr = get_core_data() as *mut CPULocalData;
    init_timer_daemon(data_ptr);

    let cr3 = get_cr3();
    BSP_CR3.store(cr3, Ordering::Relaxed);

    init_smp();

    time::init_realtime();
    klogln!("Initialized Real Time Clock. Current time is: {}", epoch_to_datetime(time::get_realtime()));

    init_keyboard_irq();
    enable_interrupts();

    spawn_kernel_thread(tasks::initializer as *const () as usize, 0, ThreadPriority::MAXIMUM, KERNEL_PROCESS.clone());

    terminate_thread!();
}
