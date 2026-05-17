use core::mem::forget;
use core::ops::Deref;

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
    interrupts_enabled,
};
use crate::kernel::sync::{
    MutexGuard,
    TicketLock,
};
use crate::kernel::thread::ThreadState;
use crate::kernel::thread::dispatch::wake_thread;
use crate::kernel::thread::wait::WaitQueue;

struct CondVar {
    wait_queue: TicketLock<WaitQueue>,
}

impl CondVar {
    pub const fn new() -> Self { Self { wait_queue: TicketLock::new(WaitQueue::new()) } }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        unsafe {
            disable_interrupts();
            let mut queue = self.wait_queue.lock();
            let current_thread = get_core_data().scheduler.get_current_thread();
            (*current_thread).state = ThreadState::Blocked;
            queue.push(current_thread);

            let mutex = guard.mutex;
            forget(guard);

            mutex.unlock();
            drop(queue);

            get_core_data().scheduler.schedule();

            mutex.lock()
        }
    }

    pub fn notify_one(&self) {
        let int_state = interrupts_enabled();
        disable_interrupts();
        let mut queue = self.wait_queue.lock();
        let current_thread = queue.pop();
        if !current_thread.is_null() {
            wake_thread(current_thread);
        }
        if int_state {
            enable_interrupts()
        };
    }

    pub fn notify_all(&self) {
        let int_state = interrupts_enabled();
        disable_interrupts();
        let mut queue = self.wait_queue.lock();
        loop {
            let current_thread = queue.pop();
            if current_thread.is_null() {
                break;
            } else {
                wake_thread(current_thread);
            }
        }
        if int_state {
            enable_interrupts()
        };
    }
}
