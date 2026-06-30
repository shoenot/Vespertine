use alloc::alloc::dealloc;
use core::alloc::Layout;
use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::cpu::current_core_mut;
use crate::sync::{
    Semaphore,
    TicketLock,
};
use crate::impl_queue_methods;

// deferred work func signature
type WorkFunction = fn(*mut u8);

pub struct WorkItem {
    pub func: WorkFunction,
    pub data: *mut u8, // context/arguments
    pub next: *mut WorkItem,
}

pub struct WorkItemQueue {
    pub queue_length: AtomicUsize,
    pub head: *mut WorkItem,
    pub tail: *mut WorkItem,
}

pub struct WorkQueue {
    pub items: TicketLock<WorkItemQueue>,
    pub items_ready: Semaphore,
}

impl WorkQueue {
    pub const fn new() -> Self {
        WorkQueue {
            items: TicketLock::new(WorkItemQueue { queue_length: AtomicUsize::new(0), head: null_mut(), tail: null_mut() }),
            items_ready: Semaphore::new(0),
        }
    }
}

impl_queue_methods!(WorkItemQueue, WorkItem, head, tail);

pub extern "C" fn worker_thread() -> ! {
    loop {
        current_core_mut().work_queue.items_ready.wait();

        let mut queue = current_core_mut().work_queue.items.lock();
        let item = queue.pop();
        drop(queue);

        if !item.is_null() {
            unsafe {
                ((*item).func)((*item).data);
                dealloc(item as *mut u8, Layout::new::<WorkItem>());
            }
        } else {
            continue;
        }
    }
}
