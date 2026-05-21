use core::hint::spin_loop;
use core::sync::atomic::Ordering;

use alloc::sync::Arc;

use crate::arch::{
    enable_interrupts,
    get_core_data,
};
use crate::drivers::keyboard::kbd_processor_thread;
use crate::kernel::object::handle::HandleID;
use crate::kernel::object::invoke::{Invocation, InvocationError};
use crate::kernel::object::models::channel::init_ipc_pipeline;
use crate::kernel::object::op::FileOp;
use crate::kernel::object::vfs::{kernel_invoke, kernel_walk};
use crate::kernel::process::pcb::ProcessControlBlock;
use crate::kernel::shell::kernel_shell_thread;
use crate::kernel::thread::dispatch::{spawn_kernel_thread, spawn_user_thread};
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::reap::reaper_daemon;
use crate::kernel::time;
use crate::kernel::time::datetime::epoch_to_datetime;
use crate::kernel::time::sleep;
use crate::memory::HHDMOFFSET;
use crate::memory::vmm::{VM_FLAG_EXEC, VM_FLAG_USER, VM_FLAG_WRITE};
use crate::memory::vmo::{PagedBackingStore, Vmo};
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

    let file_handle = kernel_walk("/docs/filetest.txt", HandleID(0)).expect("File not found!");
    let mut buf = [0u8; 64];

    let read_op = FileOp::Read { offset: 0, buffer_ptr: buf.as_mut_ptr(), len: buf.len() };
    let bytes_read = kernel_invoke(file_handle, Invocation::File(read_op)).expect("Failed to read");

    klogln!("Ramdisk read success: {}", core::str::from_utf8(&buf[..bytes_read]).unwrap());

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
    // let user_proc = ProcessControlBlock::new();
    //
    // let code_vmo = Vmo::new(4096);
    // let phys_page = code_vmo.request_page(0).unwrap();
    // let virt_page = phys_page + *HHDMOFFSET;
    // let test_code = [0xEB, 0xFE];
    // unsafe {
    //     core::ptr::copy_nonoverlapping(test_code.as_ptr(), virt_page as *mut u8, test_code.len());
    // }
    //
    // let code_addr = match user_proc.vmm.write()
    //     .mmap_vmo(4096, VM_FLAG_USER | VM_FLAG_EXEC, code_vmo as Arc<dyn PagedBackingStore>) {
    //     Some(addr) => addr,
    //     None => panic!("user code alloc failed"),
    // };
    //
    // let stack_size = 8192;
    // let stack_addr = match user_proc.vmm.write().mmap(stack_size, VM_FLAG_USER | VM_FLAG_WRITE) {
    //     Some(addr) => addr,
    //     None => panic!("user code alloc failed"),
    // };
    // let user_stack_top = stack_addr + stack_size;
    //
    // spawn_user_thread(code_addr, user_stack_top, 0, ThreadPriority::MEDIUM, user_proc);
