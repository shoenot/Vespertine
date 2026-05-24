pub mod callout;
mod clock;
pub mod datetime;

use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicBool,
    AtomicPtr,
};

pub use clock::*;

use crate::arch::x86_64::timer;
use crate::core::sync::{
    KernelOnceCell,
    TicketLock,
};

pub static TIME_SRC_FQ: KernelOnceCell<usize> = KernelOnceCell::new();
pub static LAPIC_FQ: KernelOnceCell<usize> = KernelOnceCell::new();
pub static HPET_BASE_ADDR: AtomicPtr<usize> = AtomicPtr::new(null_mut());

pub type TimeFn = extern "sysv64" fn() -> usize;
pub static GET_TIME_FN: AtomicPtr<()> = AtomicPtr::new(null_mut());

pub static USE_TSC_DEADLINE: AtomicBool = AtomicBool::new(false);
const IA32_TSC_DEADLINE: u32 = 0x6E0;

pub enum TimeSource {
    None,
    TSC(timer::tsc::TSC),
    HPET(timer::hpet::HPET),
}

pub static TIME_SOURCE: TicketLock<TimeSource> = TicketLock::new(TimeSource::None);

pub trait ClockSource {
    fn name(&self) -> &'static str;
    fn read_counter(&self) -> usize;
    fn frequency(&self) -> usize;
}

pub fn init() { crate::arch::x86_64::timer::init(); }
