use alloc::alloc::alloc;
use core::alloc::Layout;
use core::ptr::write_volatile;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use hal::ipi::send_reschedule_ipi;
use hal::context::init_thread_stack;
use crate::cpu::{
    NUM_CORES, current_core_id, current_core_mut, get_core_data_for, try_get_core_data_for
};
use crate::process::Process;
use crate::sched::priority::ThreadPriority;
use crate::sched::{
    ThreadBlockState,
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
        (*tcb_ptr).set_assigned_core(best_core);
    }

    let this_core = current_core_id();
    let target_data = get_core_data_for(best_core);

    if best_core == this_core {
        target_data.scheduler.mailbox.lock().push(tcb_ptr);
    } else {
        let mut mailbox = target_data.scheduler.mailbox.lock();
        mailbox.push(tcb_ptr);
        drop(mailbox);

        send_reschedule_ipi(best_core);
    }
    tcb_ptr
}

pub fn spawn_user_thread(
    entry_point: usize, user_stack_top: usize, arg: usize, priority: ThreadPriority, proc: Process,
) -> *mut ThreadControlBlock {
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
        (*tcb_ptr).set_assigned_core(best_core);
    }

    let this_core = current_core_id();
    let target_data = get_core_data_for(best_core);

    if best_core == this_core {
        target_data.scheduler.mailbox.lock().push(tcb_ptr);
    } else {
        let mut mailbox = target_data.scheduler.mailbox.lock();
        mailbox.push(tcb_ptr);
        drop(mailbox);

        send_reschedule_ipi(best_core);
    }
    tcb_ptr
}

pub fn create_user_thread_suspended(
    entry_point: usize, user_stack_top: usize, arg: usize, priority: ThreadPriority, proc: Process,
) -> *mut ThreadControlBlock {
    create_user_tcb(entry_point, user_stack_top, arg, priority, proc).expect("Unable to create suspended user thread")
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
        (*tcb_ptr).init(switch_addr, stack_base, stack_size, fpu_ptr, 0, priority, proc.clone());
    }

    proc.register_thread(tcb_ptr);
    proc.active_threads.fetch_add(1, Ordering::SeqCst);

    Ok(tcb_ptr)
}

pub fn create_user_tcb(
    entry_point: usize, user_stack_top: usize, arg: usize, priority: ThreadPriority, proc: Process,
) -> Result<*mut ThreadControlBlock, ThreadError> {
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
        (*tcb_ptr).init(switch_addr, stack_base, stack_size, fpu_ptr, 0, priority, proc.clone());
    }

    proc.register_thread(tcb_ptr);
    proc.active_threads.fetch_add(1, Ordering::SeqCst);

    Ok(tcb_ptr)
}

pub fn reschedule_thread_core(thread: *mut ThreadControlBlock) {
    if thread.is_null() {
        return;
    }
    unsafe {
        let tgt_core = (*thread).assigned_core();
        let this_core = current_core_id();

        if tgt_core != this_core {
            send_reschedule_ipi(tgt_core);
        }
    }
}

pub fn enqueue_ready_thread(thread: *mut ThreadControlBlock) {
    if thread.is_null() {
        return;
    }

    unsafe {
        let this_core = current_core_id();
        let target_core = (*thread).assigned_core();

        if this_core == target_core {
            current_core_mut().scheduler.push(thread);
        } else {
            let target_data = get_core_data_for(target_core);
            let mut mailbox = target_data.scheduler.mailbox.lock();
            mailbox.push(thread);
            drop(mailbox);
            send_reschedule_ipi(target_core);
        }
    }
}

pub fn try_wake_thread(thread: *mut ThreadControlBlock) -> bool {
    unsafe {
        if (*thread).state() == ThreadState::Terminated {
            return false;
        }

        if (*thread).transition(ThreadState::Blocked, ThreadState::Ready).is_err() {
            return false;
        }

        (*thread).clear_block_state();
        (*thread).effective_priority = (*thread).base_priority.boosted(2);
        true
    }
}

pub fn cancel_block_if_awoken(thread: &ThreadControlBlock, awoken: &AtomicBool) -> bool {
    if awoken.swap(false, Ordering::AcqRel) && thread.transition(ThreadState::Blocked, ThreadState::Running).is_ok() {
        thread.clear_block_state();
        true
    } else {
        false
    }
}

pub fn wake_thread(thread: *mut ThreadControlBlock) {
    if !try_wake_thread(thread) {
        return;
    }
    enqueue_ready_thread(thread);
}

pub fn start_thread(thread: *mut ThreadControlBlock) { enqueue_ready_thread(thread); }

pub fn cancel_blocked_thread(thread: *mut ThreadControlBlock) -> bool {
    if thread.is_null() {
        return false;
    }

    unsafe {
        (*thread).request_cancel();

        if (*thread).state() != ThreadState::Blocked {
            return false;
        }

        let state = (*thread).take_block_state();

        let removed = match state {
            ThreadBlockState::None => false,
            ThreadBlockState::WaitQueue { queue } => {
                if queue.is_null() {
                    false
                } else {
                    (*queue).lock().remove(thread)
                }
            }
            ThreadBlockState::Registration { registration } => registration.cancel(),
            ThreadBlockState::Futex { addr } => {
                let proc = (*thread).process.clone();
                let mut futexes = proc.futexes.write();

                match futexes.get_mut(&addr) {
                    Some(queue) => queue.remove(thread),
                    None => false,
                }
            }
        };

        if !removed {
            return false;
        }

        if (*thread).transition(ThreadState::Blocked, ThreadState::Ready).is_err() {
            return false;
        }

        enqueue_ready_thread(thread);
        true
    }
}
