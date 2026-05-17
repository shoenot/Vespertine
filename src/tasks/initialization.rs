use core::sync::atomic::Ordering;

use crate::arch::{
    enable_interrupts,
    get_core_data,
};
use crate::drivers::keyboard::kbd_processor_thread;
use crate::kernel::thread::dispatch::spawn_kernel_thread;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::reap::reaper_daemon;
use crate::kernel::time;
use crate::kernel::time::datetime::epoch_to_datetime;
use crate::kernel::time::sleep;
use crate::tests::smp_tests::{
    MUTEX_RACE,
    THREADS_FINISHED,
    consumer_thread,
    contention_mutex_thread,
    producer_thread,
};
use crate::{
    klogln,
    terminate_thread,
};

// Kernel initialization tasks

// Init function dispatcher
pub extern "C" fn initializer(_arg: usize) -> ! {
    spawn_kernel_thread(reaper_daemon as *const () as usize, 0, ThreadPriority::REAPER);
    spawn_kernel_thread(kbd_processor_thread as *const () as usize, 0, ThreadPriority::HIGH);

    terminate_thread!();
}

pub extern "C" fn watchdog(threads: usize) -> ! {
    loop {
        if THREADS_FINISHED.load(Ordering::Relaxed) == threads {
            let guard = MUTEX_RACE.lock();
            let counter = *guard;
            drop(guard);
            klogln!("All threads finished. Final count: {}", counter);
            break;
        } else {
            sleep(1_000_000_000);
        }
    }
    terminate_thread!();
}

pub extern "C" fn time_print_dispatcher(_arg: usize) -> ! {
    loop {
        spawn_kernel_thread(time_print as *const () as usize, 0, ThreadPriority::MEDIUM);
        sleep(1_000_000_000);
    }
}

pub extern "C" fn time_print(_arg: usize) -> ! {
    enable_interrupts();
    klogln!("Current time is: {}", epoch_to_datetime(time::get_realtime()));
    terminate_thread!();
}
