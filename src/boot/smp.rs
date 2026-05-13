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
    CPULocalData,
    activate_core,
    get_core_data,
};
use crate::arch::x86_64::interrupts::enable_interrupts;
use crate::arch::x86_64::interrupts::idt::load_idt;
use crate::kernel::time::USE_TSC_DEADLINE;
use crate::klogln;
use crate::memory::paging::load_cr3;

pub static BSP_CR3: AtomicU64 = AtomicU64::new(0);

pub extern "C" fn ap_entry(mp_info: &MpInfo) -> ! {
    load_cr3(BSP_CR3.load(Ordering::Relaxed));
    let core_data_ptr = mp_info.extra_argument() as *mut CPULocalData;
    activate_core(core_data_ptr);

    load_idt();
    init_fpu(false);

    let core_data = get_core_data();

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

    klogln!("Started {}", get_core_data().lapic_id);
    enable_interrupts();
    core_data.scheduler.terminate();
    unreachable!();
}
