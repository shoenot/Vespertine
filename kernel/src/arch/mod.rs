pub mod x86_64;

use hal::cpu::cpuid::check_xsave_support;
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
pub use x86_64::{
    init_global_apics,
    init_interrupts,
};

use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::cpu::fpu::init_xsave;
use crate::arch::x86_64::timer::read_rtc;
use crate::core::cpu::KernelCoreData;
use crate::core::time::datetime::datetime_to_epoch;

pub fn init() { init_interrupts(); }

pub fn init_bootstrap_core() {
    let lapic = init_local_apic();
    let lapic_id = lapic.id();
    let data_ptr = init_core_data(lapic_id as usize, 0, lapic);
    activate_core(data_ptr);
}

pub fn init_fpu(bsp: bool) {
    unsafe {
        init_cr4();
    }
    if bsp {
        init_default_fpu_cxt();
    } else if check_xsave_support() {
        unsafe {
            init_xsave();
        }
    }
}

pub fn get_rtc_unix_timestamp() -> i64 { datetime_to_epoch(read_rtc()) }

pub fn current_kernel_core_data() -> *mut KernelCoreData {
    x86_64::cpu::core::current_kernel_core_data()
}

pub fn send_reschedule_ipi(logical_id: usize) {
    x86_64::apic::lapic::send_reschedule_ipi(logical_id);
}

pub fn send_tlb_shootdown_ipi(logical_id: usize) {
    x86_64::apic::lapic::send_tlb_shootdown_ipi(logical_id);
}

pub fn msi_message_fields_for_target(core_logical_id: usize, vector: u8) -> (u32, u32, u32) {
    crate::arch::x86_64::apic::lapic::msi_message_fields_for_target(core_logical_id, vector)
}

pub fn arm_local_timer_oneshot(ticks: u32) {
    x86_64::timer::arm_local_timer_oneshot(ticks);
}

pub fn stop_local_timer() {
    x86_64::timer::stop_local_timer();
}

pub fn set_kernel_stack(stack_top: usize) {
    x86_64::task::set_kernel_stack(stack_top);
}
