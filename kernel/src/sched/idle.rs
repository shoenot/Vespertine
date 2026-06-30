use hal::context::init_bootstrap_thread_stack;

use crate::cpu::hal_boot_alloc;
use crate::sched::Thread;
use crate::sched::priority::ThreadPriority;
use crate::{
    BOOTSTRAP_ALLOC,
    KERNEL_PROCESS,
};


fn idle_loop() -> ! {
    loop {
        hal::cpu::idle();
    }
}

pub fn init_idle_thread(core_logical_id: usize) -> *mut Thread {
    let stack_size = 4096 * 16;

    let tcb_ptr = BOOTSTRAP_ALLOC.lock().alloc(size_of::<Thread>(), 8) as *mut Thread;
    let stack_base = BOOTSTRAP_ALLOC.lock().alloc(stack_size, 8) as usize;

    let idle_loop_addr = idle_loop as *const () as usize;
    let (switch_addr, extended_context) = init_bootstrap_thread_stack(idle_loop_addr, 0, stack_base, stack_size, hal_boot_alloc);

    // init TCB
    unsafe {
        (*tcb_ptr).init(
            switch_addr,
            stack_base,
            stack_size,
            extended_context,
            core_logical_id,
            ThreadPriority::IDLE,
            KERNEL_PROCESS.clone(),
        );
        (*tcb_ptr).base_priority = ThreadPriority::IDLE;
        (*tcb_ptr).effective_priority = ThreadPriority::IDLE;
    }

    tcb_ptr
}
