use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use limine::mp::MpInfo;

use crate::arch::init_fpu;
use crate::arch::x86_64::apic::lapic::{
    ApicDriver,
    ApicMode,
    TimerMode,
};
use crate::arch::x86_64::cpu::core::{
    activate_core,
    get_core_data,
    CPULocalData,
};
use crate::arch::x86_64::interrupts::enable_interrupts;
use crate::arch::x86_64::interrupts::idt::load_idt;
use crate::core::time::USE_TSC_DEADLINE;
use crate::memory::paging::load_cr3;
use crate::terminate_thread;

pub static BSP_CR3: AtomicU64 = AtomicU64::new(0);

pub extern "C" fn ap_entry(mp_info: &MpInfo) -> ! {
    let core_data_ptr = mp_info.extra_argument() as *mut CPULocalData;
    load_cr3(BSP_CR3.load(Ordering::Relaxed));
    load_idt();
    activate_core(core_data_ptr);
    crate::arch::x86_64::cpu::core::init_timer_daemon(core_data_ptr);

    let core_data = get_core_data();
    let logical_id = core_data.logical_id;
    core_data.scheduler.init_threads(logical_id);

    init_fpu(false);

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
