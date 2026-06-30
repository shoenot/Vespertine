use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use hal::cpu::{
    CpuLocalPtr,
    kernel_core_from_cpu_local,
};
use hal::mmu::load_cr3;

use crate::cpu::{
    KernelCoreData,
    current_core_id,
    current_core_mut,
    hal_boot_alloc,
};
use crate::time::callout::init_timer_daemon;
use crate::terminate_thread;

pub static BSP_CR3: AtomicU64 = AtomicU64::new(0);

pub extern "C" fn ap_entry(cpu_local: CpuLocalPtr) -> ! {
    load_cr3(BSP_CR3.load(Ordering::Acquire));
    hal::interrupts::load_local();
    hal::cpu::activate_core(cpu_local);
    let kernel_core_data = kernel_core_from_cpu_local(cpu_local) as *mut KernelCoreData;

    let logical_id = current_core_id();
    current_core_mut().scheduler.init_threads(logical_id);

    init_timer_daemon(kernel_core_data);

    hal::cpu::init_ap_state(hal_boot_alloc);
    hal::timer::init_local();

    hal::interrupts::enable_interrupts();
    terminate_thread!();
}
