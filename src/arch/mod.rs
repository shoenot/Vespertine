pub mod x86_64;

pub use x86_64::LOCAL_APIC;

use x86_64::{
    init_interrupts,
    init_apic,
    cpu::fpu::{
        init_cr4,
        init_default_fpu_cxt,
    },
};


pub fn init() {
    init_interrupts();
}

pub fn init_timers() {
    init_apic();
}

pub fn init_fpu() {
    unsafe {
        init_cr4();
        init_default_fpu_cxt();
    }
}
