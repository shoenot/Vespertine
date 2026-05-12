#![no_std]
#![no_main]
mod arch;
mod boot;
mod drivers;
mod helpers;
mod memory;
mod kernel;
mod panic;
mod tests;

mod demo;

extern crate alloc;

pub use boot::*;
use crate::{
    demo::run_demo,
    kernel::{
        SCHEDULER,
        time,
    },
    panic::hcf,
};


#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    arch::init();
    memory::init();
    time::init();
    arch::init_fpu();

    SCHEDULER.lock().init();

    run_demo();
}
