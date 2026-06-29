use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use hal::interrupts::enable_interrupts;
use limine::mp::MpInfo;

use crate::arch::init_fpu;
use crate::arch::x86_64::apic::lapic::{
    ApicDriver,
    ApicMode,
    TimerMode,
};
use crate::arch::x86_64::cpu::core::{
    CPULocalData,
    activate_core,
};
use crate::arch::x86_64::interrupts::idt::load_idt;
use crate::core::cpu::{KernelCoreData, current_core_id, current_core_mut};
use crate::core::time::USE_TSC_DEADLINE;
use crate::core::time::callout::init_timer_daemon;
use hal::mmu::load_cr3;
use crate::terminate_thread;

pub static BSP_CR3: AtomicU64 = AtomicU64::new(0);

pub extern "C" fn ap_entry(mp_info: &MpInfo) -> ! {
    let core_data_ptr = mp_info.extra_argument() as *mut CPULocalData;
    load_cr3(BSP_CR3.load(Ordering::Acquire));
    load_idt();
    activate_core(core_data_ptr);
    let kernel_core_data = unsafe { &mut (*core_data_ptr).kernel_data as *mut KernelCoreData };

    let logical_id = current_core_id();
    current_core_mut().scheduler.init_threads(logical_id);

    init_timer_daemon(kernel_core_data);


    init_fpu(false);

    let core_data = crate::arch::get_core_data();
    match &mut core_data.apic_mode {
        ApicMode::XApic(a) => {
            a.init();
        }
        ApicMode::X2Apic(a) => {
            a.init();
        }
    }

    if USE_TSC_DEADLINE.load(Ordering::Relaxed) {
        core_data.apic_mode.timer_setup(35, 0, TimerMode::TscDeadline);
    } else {
        core_data.apic_mode.timer_setup(35, 0, TimerMode::OneShot);
    }

    enable_interrupts();
    terminate_thread!();
}
