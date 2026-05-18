use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use crate::arch::{
    enable_interrupts,
    get_core_data,
};
use crate::drivers::keyboard::kbd_processor_thread;
use crate::kernel::object::handle::AccessRights;
use crate::kernel::object::invoke::Invocation;
use crate::kernel::object::table::{
    PRINCIPAL_HANDLE_TABLE,
    TestDevice,
    debug_dump_handles,
    kernel_register_obj,
    sys_close,
    sys_duplicate,
    sys_invoke,
};
use crate::kernel::thread::dispatch::spawn_kernel_thread;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::reap::reaper_daemon;
use crate::kernel::time;
use crate::kernel::time::datetime::epoch_to_datetime;
use crate::kernel::time::sleep;
use crate::tests::smp_tests::{
    MUTEX_RACE,
    THREADS_FINISHED,
};
use crate::{
    klogln,
    terminate_thread,
    tests,
};

// Kernel initialization tasks

// Init function dispatcher
pub extern "C" fn initializer(_arg: usize) -> ! {
    tests::memory_tests::run_pmm_tests();

    let dev = Arc::new(TestDevice {});

    let handle = kernel_register_obj(dev, AccessRights::READ);

    match sys_invoke(handle, Invocation::Ping) {
        Ok(_) => {}
        Err(e) => klogln!("{}", e),
    }

    sys_close(handle).unwrap();

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
