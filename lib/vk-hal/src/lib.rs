#![no_std]

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub mod arch {
    pub use crate::x86_64::*;
}

#[cfg(target_arch = "x86_64")]
pub mod cpu {
    pub use crate::x86_64::cpu::*;
}

#[cfg(target_arch = "x86_64")]
pub mod ipi {
    pub use crate::x86_64::apic::lapic::{
        send_reschedule_ipi,
        send_tlb_shootdown_ipi,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod timer {
    pub use crate::x86_64::apic::lapic::{
        TimerMode,
        arm_local_timer_oneshot,
        setup_local_timer,
        stop_local_timer,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod interrupts {
    pub use crate::x86_64::interrupts::*;
    pub use crate::x86_64::apic::lapic::{
        send_eoi,
        msi_message_fields_for_target,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod context {
    pub use crate::x86_64::task::context::*;
}

#[cfg(target_arch = "x86_64")]
pub mod fpu {
    pub use crate::x86_64::fpu::*;
}

#[cfg(target_arch = "x86_64")]
pub mod mmu {
    pub use crate::x86_64::mmu::*;
}

#[cfg(target_arch = "x86_64")]
pub mod io {
    pub use crate::x86_64::io::*;
}

#[cfg(target_arch = "x86_64")]
pub mod msr {
    pub use crate::x86_64::msr::*;
}

#[cfg(target_arch = "x86_64")]
pub mod usercopy {
    pub use crate::x86_64::usercopy::*;
}

