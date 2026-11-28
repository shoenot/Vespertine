use alloc::alloc::{alloc, dealloc};
use alloc::sync::Arc;
use hal::interrupts::{disable_interrupts, interrupts_enabled};
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
use crate::sched::block::ThreadWakeRegistration;
use crate::sched::priority::ThreadPriority;
use crate::sched::{
    Thread, ThreadError, ThreadPin, ThreadState
};

pub fn spawn_kernel_thread(entry_point: usize, arg: usize, priority: ThreadPriority, proc: Process) -> *mut Thread {
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
    entry_point: usize, 
    user_stack_top: usize, 
    arg: usize, 
    priority: ThreadPriority, 
    proc: Process) -> *mut Thread {
    let tcb_ptr = create_user_tcb(entry_point, user_stack_top, arg, priority, proc)
        .expect("Unable to spawn user thread");

    unsafe { enqueue_new_user_thread(tcb_ptr) };
    tcb_ptr

}

pub unsafe fn enqueue_new_user_thread(tcb_ptr: *mut Thread) {
    let mut best_core = 0;
    let mut min_load = usize::MAX;

    for logical_id in 0..*NUM_CORES {
        if let Some(target_data) = try_get_core_data_for(logical_id) {
            let load = target_data.scheduler.queue_length.load(Ordering::Acquire) 
                + target_data.scheduler.mailbox.lock().queue_length.load(Ordering::Acquire);
            if load < min_load {
                min_load = load;
                best_core = logical_id;
            }
        }
    }

    unsafe { (*tcb_ptr).set_assigned_core(best_core) };

    let this_core = current_core_id();
    let target = get_core_data_for(best_core);
    target.scheduler.mailbox.lock().push(tcb_ptr);

    if best_core != this_core { send_reschedule_ipi(best_core) };
}

pub fn create_user_thread_suspended(
    entry_point: usize, user_stack_top: usize, arg: usize, priority: ThreadPriority, proc: Process,
) -> *mut Thread {
    create_user_tcb(entry_point, user_stack_top, arg, priority, proc).expect("Unable to create suspended user thread")
}

pub fn create_tcb(entry_point: usize, arg: usize, priority: ThreadPriority, proc: Process) -> Result<*mut Thread, ThreadError> {
    let stack_size = 4096 * 4;
    // alloc memory for structs
    let tcb_layout = Layout::new::<Thread>();
    let stack_layout = Layout::from_size_align(stack_size, 4096)?;

    let tcb_ptr = unsafe { alloc(tcb_layout) as *mut Thread };
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

    Ok(tcb_ptr)
}

pub fn create_user_tcb(
    entry_point: usize, user_stack_top: usize, arg: usize, priority: ThreadPriority, proc: Process,
) -> Result<*mut Thread, ThreadError> {
    let stack_size = 4096 * 4;
    let tcb_layout = Layout::new::<Thread>();
    let stack_layout = Layout::from_size_align(stack_size, 4096)?;

    let tcb_ptr = unsafe { alloc(tcb_layout) }.cast::<Thread>();
    if tcb_ptr.is_null() { return Err(ThreadError::AllocationFailed) };

    let stack = unsafe { alloc(stack_layout) };
    if stack.is_null() {
        unsafe { dealloc(tcb_ptr.cast::<u8>(), tcb_layout) };
        return Err(ThreadError::AllocationFailed);
    }
    let stack_base = stack as usize;

    unsafe {
        let stack_ptr_u64 = stack_base as *mut u64;
        for i in 0..(stack_size / 8) {
            write_volatile(stack_ptr_u64.add(i), 0);
        }
        write_volatile(tcb_ptr as *mut u8, 0);
    }

    let (switch_addr, fpu_ptr) = match init_thread_stack(entry_point, arg, stack_base, stack_size, true, user_stack_top) {
        Ok(context) => context,
        Err(error) => {
            unsafe {
                dealloc(stack, stack_layout);
                dealloc(tcb_ptr.cast::<u8>(), tcb_layout);
            }
            return Err(error.into());
        }
    };

    // init TCB
    unsafe {
        (*tcb_ptr).init(switch_addr, stack_base, stack_size, fpu_ptr, 0, priority, proc.clone());
    }

    proc.register_thread(tcb_ptr);

    Ok(tcb_ptr)
}

pub fn reschedule_thread_core(thread: *mut Thread) {
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

pub fn enqueue_ready_thread(thread: *mut Thread) {
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

pub fn try_wake_thread(ptr: *mut Thread) -> bool {
    let Some(thread) = (unsafe { ptr.as_ref() }) else { return false };
    let park = thread.block_state.lock();

    if park.is_some() || thread.transition(ThreadState::Blocked, ThreadState::Ready).is_err() { return false };

    unsafe { (*ptr).effective_priority = (*ptr).base_priority.boosted(2) };
    true
}

pub fn cancel_block_if_awoken(thread: &Thread, awoken: &AtomicBool) -> bool {
    if !awoken.swap(false, Ordering::AcqRel) { return false };

    let park = thread.block_state.lock();
    park.is_none() && thread.transition(ThreadState::Blocked, ThreadState::Running).is_ok()
}

pub(crate) fn resume_registered_if_awoken(thread: &Thread, expected: &Arc<ThreadWakeRegistration>, awoken: &AtomicBool) -> bool {
    if !awoken.swap(false, Ordering::AcqRel) { return false };

    let mut park = thread.block_state.lock();
    if !park.as_ref().is_some_and(|installed| Arc::ptr_eq(installed, expected)) || thread.transition(ThreadState::Blocked, ThreadState::Running).is_err() {
        return false;
    }

    *park = None;
    true
}

pub(crate) fn wake_registered_thread(pin: &ThreadPin, expected: &ThreadWakeRegistration) -> bool {
    let ptr = pin.as_ptr();
    let thread = unsafe { &*ptr };

    {
        let mut park = thread.block_state.lock();
        if !park.as_deref().is_some_and(|installed| core::ptr::eq(installed, expected)) || thread.transition(ThreadState::Blocked, ThreadState::Ready).is_err() {
            return false;
        }

        *park = None;
        unsafe { (*ptr).effective_priority = (*ptr).base_priority.boosted(2) };
    }

    enqueue_ready_thread(ptr);
    true
}

pub fn start_thread(thread: *mut Thread) { enqueue_ready_thread(thread); }

pub fn wake_thread(thread: *mut Thread) {
    if try_wake_thread(thread) {
        enqueue_ready_thread(thread);
    }
}

pub(crate) fn request_thread_termination(pin: &ThreadPin) {
    let ptr = pin.as_ptr();
    let thread = unsafe { &*ptr };

    if thread.state() == ThreadState::Terminated {
        return;
    }

    thread.request_cancel();

    if matches!(thread.state(), ThreadState::Running | ThreadState::Ready) {
        reschedule_thread_core(ptr);
    }
}

pub fn run_user_kernel_entry(run: impl FnOnce()) {
    let tcb = current_core_mut().scheduler.get_current_thread();
    let thread = unsafe { &* tcb };

    debug_assert!(!interrupts_enabled());
    debug_assert!(!thread.user_kernel_active.load(Ordering::Relaxed));
    thread.user_kernel_active.store(true, Ordering::Release);

    if !thread.cancel_requested() && !thread.process.is_terminating() {
        run();
    }

    disable_interrupts();
    if thread.cancel_requested() || thread.process.is_terminating() {
        current_core_mut().scheduler.terminate_current_thread(0);
    }

    thread.user_kernel_active.store(false, Ordering::Relaxed);
}
