use alloc::sync::Arc;
use vespertine_abi::op::ProcManOp;
use vespertine_abi::tag::{
    CAP_PROCMAN, CAP_SERIAL_WRITE,
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

use crate::object::invoke::InvokeContext;
use crate::cpu::current_core_mut;
use crate::drivers::serial::SerialDevice;
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
use crate::sched::dispatch::spawn_kernel_thread;
use crate::sched::priority::ThreadPriority;
use crate::sched::reap::reaper_daemon;
use crate::drivers::keyboard::kbd_processor_thread;
use crate::drivers::logger::ScreenWriter;
use crate::init::vfs_init::{init_device, init_vfs};
#[cfg(feature = "diagnostics")]
use crate::tests;
use crate::{
    KERNEL_PROCESS,
    klogln,
    terminate_thread,
};

// Kernel initialization tasks

// Init function dispatcher
pub extern "C" fn initializer(_arg: usize) -> ! {
    #[cfg(feature = "diagnostics")]
    tests::run_pre_vfs_tests();

    spawn_kernel_thread(reaper_daemon as *const () as usize, 0, ThreadPriority::REAPER, KERNEL_PROCESS.clone());

    #[cfg(feature = "diagnostics")]
    tests::thread_lifetime_tests::run();

    // socket pair for keyboard
    let (kbd_source_handle, kbd_sink_handle) = init_ipc_pipeline();
    spawn_kernel_thread(kbd_processor_thread as *const () as usize, kbd_sink_handle.0, ThreadPriority::HIGH, KERNEL_PROCESS.clone());

    spawn_kernel_thread(executor_thread as *const () as usize, 0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone());

    let executor = Executor::new();
    executor.spawn(async move {
        init_vfs().await;

        let serial_opener = init_device(
            Arc::new(SerialDevice), 
            "Serial", 
            AccessRights::READ | AccessRights::CREATE
        ).await.expect("Failed to publish /Devices/Serial");

        #[cfg(feature = "diagnostics")]
        tests::run_post_vfs_tests().await;

        let pm_broker_handle = kernel_walk("/System/Services/ProcManager", HandleID(0), AccessRights::READ, InvokeContext::kernel())
            .await
            .expect("[FATAL] No Process Manager broker found");

        let pm_handle = HandleID(
            kernel_invoke(
                pm_broker_handle,
                Invocation::Broker(BrokerOp::Request {
                    capability: CAP_PROCMAN,
                    requested_rights: AccessRights::CREATE | AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE,
                }),
                InvokeContext::kernel(),
            )
            .await
            .expect("[FATAL] Failed to request Process Manager capability"),
        );

        let _ = kernel_close(pm_broker_handle);

        // userspace init proc
        let screen_writer = Arc::new(ScreenWriter {});
        let screen_handle = kernel_register_obj(screen_writer, AccessRights::WRITE);

        // init package
        let exec_handle = kernel_walk("/System/Core/hesper", HandleID(0), AccessRights::READ | AccessRights::EXECUTE, InvokeContext::kernel())
            .await
            .expect("[FATAL] No program found");
        let root_handle = HandleID(0);
        let root_rights = AccessRights::all();
        let source = kbd_source_handle;
        let sink = screen_handle;

        let capabilities = [
            CapabilityGrant { id: serial_opener, rights: AccessRights::READ | AccessRights::CREATE, capability: CAP_SERIAL_WRITE},
        ];

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

        kernel_invoke(pm_handle, Invocation::ProcessManager(spawn_op), InvokeContext::kernel())
            .await
            .expect("Failed to spawn userspace init");
        let _ = kernel_close(serial_opener);
        klogln!(Info, "Boot", "Hesper started");
    });

    terminate_thread!();
}
