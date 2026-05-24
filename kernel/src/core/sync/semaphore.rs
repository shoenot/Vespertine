use core::fmt::Debug;
use core::sync::atomic::{
    AtomicIsize,
    Ordering,
};

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
    interrupts_enabled,
};
use crate::core::sync::TicketLock;
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::wait::WaitQueue;
use crate::core::thread::ThreadState;

pub struct Semaphore {
    counter: AtomicIsize,
    wait_queue: TicketLock<WaitQueue>,
}

impl Debug for Semaphore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Semaphore counter: {:?}", self.counter)
    }
}

unsafe impl Sync for Semaphore {}
unsafe impl Send for Semaphore {}

impl Semaphore {
    pub const fn new(counter: isize) -> Self { Self { counter: AtomicIsize::new(counter), wait_queue: TicketLock::new(WaitQueue::new()) } }

    pub fn wait(&self) {
        let mut counter = self.counter.load(Ordering::Relaxed);
        loop {
            if counter > 0 {
                match self.counter.compare_exchange_weak(counter, counter - 1, Ordering::Acquire, Ordering::Relaxed) {
                    Ok(_) => {
                        return;
                    }
                    Err(v) => {
                        counter = v;
                    }
                }
            } else {
                disable_interrupts();
                let sched = &mut get_core_data().scheduler;
                let mut wq = self.wait_queue.lock();

                let current = self.counter.load(Ordering::Acquire);
                if current > 0 {
                    drop(wq);
                    enable_interrupts();
                    counter = current;
                    continue;
                }

                let current_thread = sched.get_current_thread();
                unsafe {
                    (*current_thread).state = ThreadState::Blocked;
                }
                wq.push(current_thread);
                drop(wq);

                // yield CPU
                sched.schedule();

                // continue here when unlocked
                enable_interrupts();
                counter = self.counter.load(Ordering::Relaxed);
            }
        }
    }

    pub fn signal(&self) {
        self.counter.fetch_add(1, Ordering::Release);

        let ir_state = interrupts_enabled();
        disable_interrupts();

        let mut wq = self.wait_queue.lock();
        let next_thread = wq.pop();
        drop(wq);

        if !next_thread.is_null() {
            wake_thread(next_thread);
        }

        if ir_state {
            enable_interrupts();
        }
    }
}
