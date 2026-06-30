use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use hal::cpu::kernel_core_from_cpu_local;
use limine::mp::MpInfo;

use crate::core::cpu::{KernelCoreData, current_core_id, current_core_mut, hal_boot_alloc};
use crate::core::time::callout::init_timer_daemon;
use hal::mmu::load_cr3;
use crate::terminate_thread;

pub static BSP_CR3: AtomicU64 = AtomicU64::new(0);

pub extern "C" fn ap_entry(mp_info: &MpInfo) -> ! {
    let cpu_local = mp_info.extra_argument() as *mut ();
    load_cr3(BSP_CR3.load(Ordering::Acquire));
    hal::interrupts::load_local();
    hal::cpu::activate_core(cpu_local);
    let kernel_core_data = kernel_core_from_cpu_local(cpu_local) as *mut KernelCoreData;

    let logical_id = current_core_id();
    current_core_mut().scheduler.init_threads(logical_id);

    init_timer_daemon(kernel_core_data);


    hal::fpu::init_fpu(false, hal_boot_alloc);
    hal::timer::init_local();
    hal::interrupts::enable_interrupts();
    terminate_thread!();
}
