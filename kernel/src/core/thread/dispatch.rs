use alloc::alloc::alloc;
use core::alloc::Layout;
use core::ptr::write_volatile;
use core::sync::atomic::Ordering;

use crate::arch::get_core_data;
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::task::context::init_thread_stack;
use crate::core::cpu::{
    NUM_CORES,
    get_core_data_for,
    try_get_core_data_for,
};
use crate::core::object::models::process::Process;
use crate::core::thread::priority::ThreadPriority;
use crate::core::thread::{
    ThreadControlBlock,
    ThreadError,
    ThreadState,
};

pub fn spawn_kernel_thread(entry_point: usize, arg: usize, priority: ThreadPriority, proc: Process) -> *mut ThreadControlBlock {
    let tcb_ptr = create_tcb(entry_point, arg, priority, proc).expect("Unable to spawn kernel thread");

    let mut best_core = 0;
    let mut min_load = usize::MAX;

    for logical_id in 0..*NUM_CORES {
        if let Some(target_data) = try_get_core_data_for(logical_id) {
            let load = target_data.scheduler.queue_length.load(Ordering::Acquire) +
                target_data.scheduler.mailbox.lock().queue_length.load(Ordering::Acquire);
            if load < min_load {
                min_load = load;
                best_core = logical_id;
            }
        }
    }

    unsafe {
        (*tcb_ptr).home_core = best_core;
    }

    let this_core = get_core_data().logical_id;
    let target_data = get_core_data_for(best_core);

    if best_core == this_core {
        target_data.scheduler.mailbox.lock().push(tcb_ptr);
    } else {
        let mut mailbox = target_data.scheduler.mailbox.lock();
        mailbox.push(tcb_ptr);
        drop(mailbox);

        get_core_data().apic_mode.send_ipi(target_data.lapic_id as u32, 64);
    }
    tcb_ptr
}

pub fn spawn_user_thread(entry_point: usize, user_stack_top: usize, arg: usize, priority: ThreadPriority, proc: Process) -> *mut ThreadControlBlock {
    let tcb_ptr = create_user_tcb(entry_point, user_stack_top, arg, priority, proc).expect("Unable to spawn user thread");

    let mut best_core = 0;
    let mut min_load = usize::MAX;

    for logical_id in 0..*NUM_CORES {
        if let Some(target_data) = try_get_core_data_for(logical_id) {
            let load = target_data.scheduler.queue_length.load(Ordering::Acquire) +
                target_data.scheduler.mailbox.lock().queue_length.load(Ordering::Acquire);
            if load < min_load {
                min_load = load;
                best_core = logical_id;
            }
        }
    }

    unsafe {
        (*tcb_ptr).home_core = best_core;
    }

    let this_core = get_core_data().logical_id;
    let target_data = get_core_data_for(best_core);

    if best_core == this_core {
        target_data.scheduler.mailbox.lock().push(tcb_ptr);
    } else {
        let mut mailbox = target_data.scheduler.mailbox.lock();
        mailbox.push(tcb_ptr);
        drop(mailbox);

        get_core_data().apic_mode.send_ipi(target_data.lapic_id as u32, 64);
    }
    tcb_ptr
}

pub fn wake_thread(thread: *mut ThreadControlBlock) {
    unsafe {
        (*thread).state = ThreadState::Ready;

        let this_core = get_core_data().logical_id;
        let target_core = (*thread).home_core;

        if this_core == target_core {
            // local thread wakeup
            get_core_data().scheduler.push(thread);
        } else {
            let target_data = get_core_data_for(target_core);

            let mut mailbox = target_data.scheduler.mailbox.lock();
            mailbox.push(thread);
            drop(mailbox);

            get_core_data().apic_mode.send_ipi(target_data.lapic_id as u32, 64);
        }
    }
}

pub fn create_tcb(entry_point: usize, arg: usize, priority: ThreadPriority, proc: Process) -> Result<*mut ThreadControlBlock, ThreadError> {
    let stack_size = 4096 * 4;
    // alloc memory for structs
    let tcb_layout = Layout::new::<ThreadControlBlock>();
    let stack_layout = Layout::from_size_align(stack_size, 4096)?;

    let tcb_ptr = unsafe { alloc(tcb_layout) as *mut ThreadControlBlock };
    let stack_base = unsafe { alloc(stack_layout) as usize };

    unsafe {
        let stack_ptr_u64 = stack_base as *mut u64;
        for i in 0..(stack_size / 8) {
            write_volatile(stack_ptr_u64.add(i), 0);
        }
        write_volatile(tcb_ptr as *mut u8, 0);
    }

    let (switch_addr, fpu_ptr) = init_thread_stack(entry_point, arg, stack_base, stack_size, false, 0)?;

    // init TCB
    unsafe {
        (*tcb_ptr).init(switch_addr, stack_base, stack_size, fpu_ptr, 0, priority, proc);
    }

    Ok(tcb_ptr)
}

pub fn create_user_tcb(entry_point: usize, user_stack_top: usize, arg: usize, priority: ThreadPriority, proc: Process) -> Result<*mut ThreadControlBlock, ThreadError> {
    let stack_size = 4096 * 4;
    let tcb_layout = Layout::new::<ThreadControlBlock>();
    let stack_layout = Layout::from_size_align(stack_size, 4096)?;

    let tcb_ptr = unsafe { alloc(tcb_layout) as *mut ThreadControlBlock };
    let stack_base = unsafe { alloc(stack_layout) as usize };

    unsafe {
        let stack_ptr_u64 = stack_base as *mut u64;
        for i in 0..(stack_size / 8) {
            write_volatile(stack_ptr_u64.add(i), 0);
        }
        write_volatile(tcb_ptr as *mut u8, 0);
    }

    let (switch_addr, fpu_ptr) = init_thread_stack(entry_point, arg, stack_base, stack_size, true, user_stack_top)?;

    // init TCB
    unsafe {
        (*tcb_ptr).init(switch_addr, stack_base, stack_size, fpu_ptr, 0, priority, proc);
    }

    Ok(tcb_ptr)
}
