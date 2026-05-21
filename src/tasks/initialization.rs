use core::sync::atomic::Ordering;

use crate::arch::{
    enable_interrupts,
    get_core_data,
};
use crate::drivers::keyboard::kbd_processor_thread;
use crate::kernel::object::models::channel::init_ipc_pipeline;
use crate::kernel::object::vfs::{debug_dump_handles, kernel_register_obj};
use crate::kernel::shell::kernel_shell_thread;
use crate::kernel::thread::dispatch::spawn_kernel_thread;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::reap::reaper_daemon;
use crate::kernel::time;
use crate::kernel::time::datetime::epoch_to_datetime;
use crate::kernel::time::sleep;
use crate::tasks::vfs_init::init_vfs;
use crate::tests::smp_tests::{
    MUTEX_RACE,
    THREADS_FINISHED,
};
use crate::{
    KERNEL_PROCESS, klogln, terminate_thread, tests
};

// Kernel initialization tasks

// Init function dispatcher
pub extern "C" fn initializer(_arg: usize) -> ! {
    tests::memory_tests::run_pmm_tests();

    init_vfs();

    spawn_kernel_thread(reaper_daemon as *const () as usize, 0, ThreadPriority::REAPER, KERNEL_PROCESS.clone());

    let (kbd_handle, shell_handle) = init_ipc_pipeline();

    spawn_kernel_thread(kbd_processor_thread as *const () as usize, kbd_handle.0, ThreadPriority::HIGH, KERNEL_PROCESS.clone());
    spawn_kernel_thread(kernel_shell_thread as *const () as usize, shell_handle.0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone());

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
        spawn_kernel_thread(time_print as *const () as usize, 0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone());
        sleep(1_000_000_000);
    }
}

pub extern "C" fn time_print(_arg: usize) -> ! {
    enable_interrupts();
    klogln!("Current time is: {}", epoch_to_datetime(time::get_realtime()));
    terminate_thread!();
}
