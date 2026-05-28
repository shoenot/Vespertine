use core::hint::spin_loop;
use core::ptr::null;
use core::sync::atomic::Ordering;

use crate::arch::{
    enable_interrupts,
    get_core_data,
};

use crate::core::asynchronous::executor_thread;
use crate::drivers::logger::{LOGGER, LogBuffer, LogTarget};
use alloc::sync::Arc;
use vespertine_abi::tag::{TAG_SYS_PROCMAN, TAG_SYS_SOCKFAC};
use vespertine_abi::{AccessRights, HandleGrant, HandleID, Invocation};
use crate::core::object::models::socket::init_ipc_pipeline;
use crate::core::object::vfs::{kernel_invoke, kernel_register_obj, kernel_walk, mount_kernel_dir};
use crate::core::thread::dispatch::spawn_kernel_thread;
use crate::core::thread::priority::ThreadPriority;
use crate::core::thread::reap::reaper_daemon;
use crate::core::time;
use crate::core::time::datetime::epoch_to_datetime;
use crate::core::time::sleep;
use crate::drivers::keyboard::kbd_processor_thread;
use crate::tasks::vfs_init::init_vfs;
use crate::tests::smp_tests::{
    MUTEX_RACE,
    THREADS_FINISHED,
};
use crate::{
    KERNEL_PROCESS, klogln, terminate_thread, tests
};
use vespertine_abi::op::{FileOp, ProcManOp};

// Kernel initialization tasks

// Init function dispatcher
pub extern "C" fn initializer(_arg: usize) -> ! {
    tests::run_pre_vfs_tests();

    init_vfs();

    tests::run_post_vfs_tests();

    spawn_kernel_thread(reaper_daemon as *const () as usize, 0, ThreadPriority::REAPER, KERNEL_PROCESS.clone());

    let console_handle = kernel_walk("/System/Services/ConsoleWriter", HandleID(0)).expect("[FATAL] No ConsoleWriter found");

    // socket pair for keyboard
    let (kbd_source_handle, kbd_sink_handle) = init_ipc_pipeline();
    spawn_kernel_thread(kbd_processor_thread as *const () as usize, kbd_sink_handle.0, ThreadPriority::HIGH, KERNEL_PROCESS.clone());

    spawn_kernel_thread(executor_thread as *const () as usize, 0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone());
    klogln!("[INFO] Launched async executor thread.");

    let file_handle = kernel_walk("/Documents/filetest.txt", HandleID(0)).expect("[FATAL] File not found!");
    let mut buf = [0u8; 64];

    let read_op = FileOp::Read { offset: 0, buffer_ptr: buf.as_mut_ptr(), len: buf.len() };
    let bytes_read = kernel_invoke(file_handle, Invocation::File(read_op)).expect("[FATAL] Failed to read from ramdisk");

    klogln!("[SUCCESS] Ramdisk read success: {}", core::str::from_utf8(&buf[..bytes_read]).unwrap());

    let pm_handle = kernel_walk("/System/Services/ProcessManager", HandleID(0)).expect("[FATAL] No Process Manager found");
    let sf_handle = kernel_walk("/System/Services/SocketFactory", HandleID(0)).expect("[FATAL] No Socket Factory found");

    // userspace init proc

    // init package 
    let exec_handle = kernel_walk("/Programs/hesper", HandleID(0)).expect("[FATAL] No program found");
    let root_handle = HandleID(0);
    let root_rights = AccessRights::all();
    let source = kbd_source_handle;
    let sink = console_handle;
    let extra_handles = [
        HandleGrant { id: pm_handle, rights: AccessRights::all(), tag: TAG_SYS_PROCMAN, },
        HandleGrant { id: sf_handle, rights: AccessRights::all(), tag: TAG_SYS_SOCKFAC, },
    ]; 

    let spawn_op = ProcManOp::Spawn { 
        exec_handle, root_handle, root_rights, source, sink,
        extra_handles_ptr: extra_handles.as_ptr(),
        extra_handles_len: extra_handles.len(),
        args_buffer_ptr: null(),
        args_buffer_len: 0,
    };

    let child_handle_id = kernel_invoke(pm_handle, Invocation::ProcessManager(spawn_op))
        .expect("[FATAL] Failed to spawn process");

    klogln!("[SUCCESS] Process spawn success. Handle: {}", child_handle_id);

    let log_buffer = Arc::new(LogBuffer::new());
    let logs_dir_handle = kernel_walk("/System/Logs", HandleID(0)).expect("[FATAL] Could not find Logs directory");
    let log_handle = kernel_register_obj(log_buffer.clone(), AccessRights::READ);
    mount_kernel_dir("kernel.log", log_handle, logs_dir_handle);

    LOGGER.lock().target = LogTarget::Buffer(log_buffer);

    klogln!("[INFO] Logger switched to log file");

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

pub extern "C" fn test_userspace(_arg: usize) -> ! {
    loop {
        spin_loop();
    }
}
