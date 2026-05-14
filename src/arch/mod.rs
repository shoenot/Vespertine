pub mod x86_64;

use core::sync::atomic::Ordering;

use x86_64::apic::lapic::init_local_apic;
pub use x86_64::cpu::core::get_core_data;
use x86_64::cpu::core::{
    activate_core,
    init_core_data,
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

pub fn init() { init_interrupts(); }

pub fn init_bootstrap_core() {
    init_global_apics();
    let lapic = init_local_apic();
    let lapic_id = lapic.id();
    let data_ptr = init_core_data(lapic_id as usize, lapic);
    activate_core(data_ptr);
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

pub fn get_unix_timestamp() -> i64 {
    let (s, mi, h, d, mo, y_u16) = read_rtc();
    
    let y = y_u16 as i64; 
    
    const DAYS_BEFORE_MONTH: [i64; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    let m = mo as usize;
    let day_val = d as i64;

    let years_since_1970 = y - 1970;
    let leap_days = (y - 1969) / 4 - (y - 1901) / 100 + (y - 1601) / 400;

    let mut days_this_year = DAYS_BEFORE_MONTH[m] + (day_val - 1);

    let is_leap_year = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
    if is_leap_year && m > 2 {
        days_this_year += 1;
    }

    let total_days = (years_since_1970 * 365) + leap_days + days_this_year;

    let total_seconds = (total_days * 86400) 
        + (h as i64 * 3600) 
        + (mi as i64 * 60) 
        + (s as i64);

    total_seconds
}
