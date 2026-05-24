use core::hint::spin_loop;
use core::sync::atomic::Ordering;

use alloc::sync::Arc;

use crate::arch::{
    enable_interrupts,
    get_core_data,
};
use crate::drivers::keyboard::kbd_processor_thread;
use crate::core::object::handle::{AccessRights, HandleID};
use crate::core::object::invoke::{Invocation, InvocationError};
use crate::core::object::models::channel::init_ipc_pipeline;
use mnemosyne_abi::op::{DirectoryOp, FileOp, MemManOp, MemPoolOp, ProcManOp};
use crate::core::object::vfs::{ROOT_DIRECTORY, kernel_close, kernel_invoke, kernel_walk, proc_cpy_handle};
use crate::core::shell::kernel_shell_thread;
use crate::core::thread::dispatch::{spawn_kernel_thread, spawn_user_thread};
use crate::core::thread::priority::ThreadPriority;
use crate::core::thread::reap::reaper_daemon;
use crate::core::time;
use crate::core::time::datetime::epoch_to_datetime;
use crate::core::time::sleep;
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

    let file_handle = kernel_walk("/Documents/filetest.txt", HandleID(0)).expect("File not found!");
    let mut buf = [0u8; 64];

    let read_op = FileOp::Read { offset: 0, buffer_ptr: buf.as_mut_ptr(), len: buf.len() };
    let bytes_read = kernel_invoke(file_handle, Invocation::File(read_op)).expect("Failed to read");

    klogln!("Ramdisk read success: {}", core::str::from_utf8(&buf[..bytes_read]).unwrap());

    klogln!("Root:");
    kernel_invoke(HandleID(0), Invocation::Directory(DirectoryOp::List(0))).expect("Cannot print root directory tree");

    let pm_handle = kernel_walk("/Objects/ProcessManager", HandleID(0)).expect("No Process Manager found");
    let exec_handle = kernel_walk("/Programs/hellotime", HandleID(0)).expect("No program found");
    let root_handle = HandleID(0);
    let root_rights = AccessRights::READ | AccessRights::WRITE;

    let spawn_op = ProcManOp::Spawn { exec_handle, root_handle, root_rights };
    let child_handle_id = kernel_invoke(pm_handle, Invocation::ProcessManager(spawn_op))
        .expect("Failed to spawn process");

    klogln!("Process spawn success. Handle: {}", child_handle_id);

    let mm_handle = kernel_walk("/Objects/MemoryManager", HandleID(0)).expect("No Memory Manager found");
    let root_pool_handle = HandleID(
        kernel_invoke(mm_handle, Invocation::MemoryManager(MemManOp::CreatePool { limit: 0 }))
        .expect("Failed to create root pool")
    );
    klogln!("Created global root pool: {:?}", root_pool_handle);

    let sub_pool_handle = HandleID(
        kernel_invoke(root_pool_handle, Invocation::MemPool(MemPoolOp::CreateSubPool { limit: 1024*1024 }))
        .expect("Failed to create sub pool")
    );
    klogln!("Created 1mb sub pool: {:?}", sub_pool_handle);

    let vmo_handle = HandleID(
        kernel_invoke(sub_pool_handle, Invocation::MemPool(MemPoolOp::AllocateVmo { size: 4096 }))
        .expect("Failed to allocate VMO")
    );
    klogln!("Allocated 4kb vmo: {:?}", vmo_handle);

    let break_attempt = kernel_invoke(sub_pool_handle, 
        Invocation::MemPool(MemPoolOp::AllocateVmo { size: 1024 * 2048 }));
    klogln!("Attempting to allocate more than sub pool limit: {:?}", break_attempt);

    klogln!("Object tests passed!");
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
