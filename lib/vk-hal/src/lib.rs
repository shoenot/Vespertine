#![no_std]

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub mod arch {
    pub use crate::x86_64::*;
}

#[cfg(target_arch = "x86_64")]
pub mod usercopy {
    pub use crate::x86_64::usercopy::*;
}
