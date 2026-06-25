use core::mem::forget;

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
    interrupts_enabled,
};
use crate::core::sync::{
    MutexGuard,
    TicketLock,
};
use crate::core::thread::schedule::ScheduleReason;
use crate::core::thread::{ThreadBlockState, ThreadState};
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::wait::WaitQueue;

struct CondVar {
    wait_queue: TicketLock<WaitQueue>,
}

impl CondVar {
    pub const fn new() -> Self { Self { wait_queue: TicketLock::new(WaitQueue::new()) } }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        unsafe {
            let int_state = interrupts_enabled();
            disable_interrupts();
            let mut queue = self.wait_queue.lock();
            let current_thread = get_core_data().scheduler.get_current_thread();
            unsafe { 
                (*current_thread).set_block_state(ThreadBlockState::WaitQueue { queue: &self.wait_queue as *const _ });
                (*current_thread).transition(ThreadState::Running, ThreadState::Blocked).expect("condvar waiter was not running")
            };
            queue.push(current_thread);

            let mutex = guard.mutex;
            forget(guard);

            mutex.unlock();
            drop(queue);

            get_core_data().scheduler.schedule(ScheduleReason::Blocked);

            if int_state {
                enable_interrupts()
            };

            mutex.lock()
        }
    }

    pub fn notify_one(&self) {
        let int_state = interrupts_enabled();
        disable_interrupts();
        let mut queue = self.wait_queue.lock();
        let current_thread = queue.pop_wakeable();
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
            let current_thread = queue.pop_wakeable();
            if current_thread.is_null() {
                break;
            } 
            wake_thread(current_thread);
        }
        if int_state {
            enable_interrupts()
        };
    }
}
