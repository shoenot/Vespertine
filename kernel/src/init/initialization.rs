use alloc::sync::Arc;
use core::hint::spin_loop;
use core::sync::atomic::Ordering;

use hal::interrupts::enable_interrupts;
use vespertine_abi::op::ProcManOp;
use vespertine_abi::tag::{
    CAP_LOGGER,
    CAP_PROCMAN,
};
use vespertine_abi::{
    AccessRights,
    BrokerOp,
    CapabilityGrant,
    HandleID,
    Invocation,
    SYSTEM_USER,
    SpawnCredentials,
};

use crate::executor::{
    Executor,
    executor_thread,
};
use crate::object::ipc::socket::init_ipc_pipeline;
use crate::object::vfs::{
    kernel_close,
    kernel_invoke,
    kernel_register_obj,
    kernel_walk,
};
use crate::cpu::current_core_mut;
use crate::sched::dispatch::spawn_kernel_thread;
use crate::sched::priority::ThreadPriority;
use crate::sched::reap::reaper_daemon;
use crate::time;
use crate::time::datetime::epoch_to_datetime;
use crate::time::sleep;
use crate::drivers::keyboard::kbd_processor_thread;
use crate::drivers::logger::ScreenWriter;
use crate::init::vfs_init::init_vfs;
use crate::tests::smp_tests::{
    MUTEX_RACE,
    THREADS_FINISHED,
};
use crate::{
    KERNEL_PROCESS,
    klogln,
    terminate_thread,
    tests,
};

// Kernel initialization tasks

// Init function dispatcher
pub extern "C" fn initializer(_arg: usize) -> ! {
    tests::run_pre_vfs_tests();

    spawn_kernel_thread(reaper_daemon as *const () as usize, 0, ThreadPriority::REAPER, KERNEL_PROCESS.clone());

    // socket pair for keyboard
    let (kbd_source_handle, kbd_sink_handle) = init_ipc_pipeline();
    spawn_kernel_thread(kbd_processor_thread as *const () as usize, kbd_sink_handle.0, ThreadPriority::HIGH, KERNEL_PROCESS.clone());

    spawn_kernel_thread(executor_thread as *const () as usize, 0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone());
    klogln!("[INFO] Launched async executor thread.");

    let executor = Executor::new();
    executor.spawn(async move {
        klogln!("[ASYNC INIT] started");
        klogln!("[ASYNC INIT] calling init_vfs()");
        init_vfs().await;
        klogln!("[ASYNC INIT] init_vfs completed");

        tests::run_post_vfs_tests().await;
        klogln!("[ASYNC INIT] post vfs tests completed");

        let pm_broker_handle = kernel_walk("/System/Services/ProcManager", HandleID(0), AccessRights::READ)
            .await
            .expect("[FATAL] No Process Manager broker found");

        let pm_handle = HandleID(
            kernel_invoke(
                pm_broker_handle,
                Invocation::Broker(BrokerOp::Request {
                    capability: CAP_PROCMAN,
                    requested_rights: AccessRights::CREATE | AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE,
                }),
            )
            .await
            .expect("[FATAL] Failed to request Process Manager capability"),
        );

        let _ = kernel_close(pm_broker_handle);

        let log_handle = kernel_walk("/System/Services/Log", HandleID(0), AccessRights::WRITE).await.expect("[FATAL] No Log Service found");

        // userspace init proc
        let screen_writer = Arc::new(ScreenWriter {});
        let screen_handle = kernel_register_obj(screen_writer, AccessRights::WRITE);

        // init package
        let exec_handle = kernel_walk("/System/Core/hesper", HandleID(0), AccessRights::READ | AccessRights::EXECUTE)
            .await
            .expect("[FATAL] No program found");
        let root_handle = HandleID(0);
        let root_rights = AccessRights::all();
        let source = kbd_source_handle;
        let sink = screen_handle;

        let capabilities = [CapabilityGrant { id: log_handle, rights: AccessRights::WRITE, capability: CAP_LOGGER }];

        let name = "Hesper";

        let spawn_op = ProcManOp::Spawn {
            name_len: name.len(),
            name_ptr: name.as_ptr() as usize,
            exec_handle,
            root_handle,
            root_rights,
            cwd_handle: root_handle,
            cwd_rights: root_rights,
            source,
            sink,
            credentials: SpawnCredentials::User { user: SYSTEM_USER },
            capabilities_ptr: capabilities.as_ptr() as usize,
            capabilities_len: capabilities.len(),
            args_buffer_ptr: 0,
            args_buffer_len: 0,
            start_suspended: false,
        };

        let child_handle_id =
            kernel_invoke(pm_handle, Invocation::ProcessManager(spawn_op)).await.expect("[FATAL] Failed to spawn process");

        klogln!("[SUCCESS] Process spawn success. Handle: {}", child_handle_id);
        klogln!("[INFO] Logger switched to log file");
    });

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
    klogln!("Current time is: {}", epoch_to_datetime(time::get_realtime().0));
    terminate_thread!();
}

pub extern "C" fn test_userspace(_arg: usize) -> ! {
    loop {
        spin_loop();
    }
}
