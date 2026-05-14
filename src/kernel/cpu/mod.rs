use limine::mp::MpGotoFunction;

use crate::arch::x86_64::cpu::core::{
    get_core_data,
    init_core_data,
};
use crate::boot::MP_REQUEST;
use crate::boot::smp::ap_entry;
use crate::demo::test_thread;

pub fn init_smp() {
    let mp_resp = MP_REQUEST.response().expect("No SMP Response from limine");
    let bsp_id = mp_resp.bsp_lapic_id;

    for core in mp_resp.cpus() {
        if core.lapic_id == bsp_id {
            continue;
        }

        unsafe {
            let ap_data_ptr = init_core_data(core.lapic_id as usize, get_core_data().apic_mode.clone());

            // let att = test_thread as *const ();
            // (*ap_data_ptr).scheduler.spawn(att as usize, core.processor_id as usize).unwrap();

            let ap_data_addr = ap_data_ptr as u64;
            let ap_entry_ptr = ap_entry as MpGotoFunction;

            core.bootstrap(ap_entry_ptr, ap_data_addr);
        }
    }
}
