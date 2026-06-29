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
pub mod interrupts {
    pub use crate::x86_64::interrupts::*;
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

