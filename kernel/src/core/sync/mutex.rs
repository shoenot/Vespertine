use core::cell::UnsafeCell;
use core::ops::{
    Deref,
    DerefMut,
};
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::arch::interrupts_enabled;
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::arch::x86_64::interrupts::{
    disable_interrupts,
    enable_interrupts,
};
use crate::core::sync::TicketLock;
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::wait::WaitQueue;
use crate::core::thread::ThreadState;

use core::fmt;

pub struct Mutex<T> {
    is_locked: AtomicBool,
    wait_queue: TicketLock<WaitQueue>,
    data: UnsafeCell<T>,
}

impl<T> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mutex")
            .field("is_locked", &self.is_locked)
            .finish_non_exhaustive()
    }
}

unsafe impl<T: Send> Sync for Mutex<T> {}
unsafe impl<T: Send> Send for Mutex<T> {}

pub struct MutexGuard<'a, T> {
    pub mutex: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Self { is_locked: AtomicBool::new(false), wait_queue: TicketLock::new(WaitQueue::new()), data: UnsafeCell::new(data) }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        loop {
            if !self.is_locked.swap(true, Ordering::Acquire) {
                return MutexGuard { mutex: self };
            }

            disable_interrupts();

            let sched = &mut get_core_data().scheduler;
            let mut wq = self.wait_queue.lock();

            // check if someone unlocked it while we were grabbing locks
            if !self.is_locked.load(Ordering::Relaxed) {
                drop(wq);
                enable_interrupts();
                continue;
            }

            let current_thread = sched.get_current_thread();
            unsafe {
                (*current_thread).state = ThreadState::Blocked;
            }
            wq.push(current_thread);

            drop(wq);

            // yield cpu
            sched.schedule();

            // continues here when unlocked
            enable_interrupts();
        }
    }

    pub fn unlock(&self) {
        self.is_locked.store(false, Ordering::Release);

        let int_state = interrupts_enabled();
        disable_interrupts();

        let mut wq = self.wait_queue.lock();
        let next_thread = wq.pop();
        drop(wq);

        if !next_thread.is_null() {
            wake_thread(next_thread);
        }

        if int_state {
            enable_interrupts()
        };
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { unsafe { &*self.mutex.data.get() } }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { unsafe { &mut *self.mutex.data.get() } }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) { self.mutex.unlock(); }
}
