use core::sync::atomic::AtomicPtr;

pub(crate) mod hpet;
pub(crate) mod tsc;

pub(crate) static GET_TIME_FN: AtomicPtr<()> = AtomicPtr::new(uninit_time as *mut ());

pub(crate) type TimeFn = fn() -> usize;

fn uninit_time() -> usize { 0 }


