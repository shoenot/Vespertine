pub mod x86_64;

use core::sync::atomic::Ordering;

use x86_64::apic::lapic::init_local_apic;
pub use x86_64::cpu::core::get_core_data;
use x86_64::cpu::core::{
    activate_core,
    init_core_data,
    init_timer_daemon,
};
use x86_64::cpu::fpu::{
    init_cr4,
    init_default_fpu_cxt,
};
pub use x86_64::hcf;
pub(crate) use x86_64::interrupts::{
    disable_interrupts,
    enable_interrupts,
    interrupts_enabled,
};
use x86_64::{
    init_global_apics,
    init_interrupts,
};

use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::cpu::fpu::{
    USE_XSAVE,
    init_xsave,
};
use crate::arch::x86_64::timer::read_rtc;
use crate::kernel::time::datetime::datetime_to_epoch;

pub fn init() { init_interrupts(); }

pub fn init_bootstrap_core() {
    let lapic = init_local_apic();
    let lapic_id = lapic.id();
    let data_ptr = init_core_data(lapic_id as usize, 0, lapic);
    activate_core(data_ptr);
    init_global_apics();
    init_timer_daemon(data_ptr);
}

pub fn init_fpu(bsp: bool) {
    unsafe {
        init_cr4();
        if bsp {
            init_default_fpu_cxt();
        } else if USE_XSAVE.load(Ordering::Relaxed) {
            init_xsave();
        }
    }
}

pub fn get_rtc_unix_timestamp() -> i64 { datetime_to_epoch(read_rtc()) }
