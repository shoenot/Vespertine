use core::ptr::null_mut;

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
};
use crate::impl_queue_methods;
use crate::kernel::sync::TicketLock;
use crate::kernel::thread::ThreadState;
use crate::kernel::thread::wait::WaitQueue;

// deferred work func signature
type WorkFunction = fn(*mut u8);

pub struct WorkItem {
    pub func: WorkFunction,
    pub data: *mut u8, // context/arguments
    pub next: *mut WorkItem,
}

pub struct WorkQueue {
    head: *mut WorkItem,
    tail: *mut WorkItem,
    wait_queue: WaitQueue,
}

impl_queue_methods!(WorkQueue, WorkItem, head, tail);

pub extern "C" fn worker_thread() -> ! {
    loop {
        disable_interrupts();
        let mut wq = get_core_data().work_queue.lock();

        if wq.head.is_null() {
            // queue is empty, go to bed
            let scheduler = &mut get_core_data().scheduler;
            let current_thread = scheduler.get_current_thread();
            unsafe { (*current_thread).state = ThreadState::Blocked };
            wq.wait_queue.push(current_thread);

            // drop lock before calling schedule
            drop(wq);
            scheduler.schedule();

            // enable interrupts after waking up
            enable_interrupts();
        } else {
            unsafe {
                let item = wq.pop();

                // again, drop lock before switching out
                drop(wq);
                enable_interrupts();

                ((*item).func)((*item).data);
            }
        }
    }
}
