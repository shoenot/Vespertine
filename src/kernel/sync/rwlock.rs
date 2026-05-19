use core::cell::UnsafeCell;
use core::fmt::Debug;
use core::ops::{
    Deref,
    DerefMut,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
    interrupts_enabled,
};
use crate::kernel::sync::TicketLock;
use crate::kernel::thread::ThreadState;
use crate::kernel::thread::dispatch::wake_thread;
use crate::kernel::thread::wait::WaitQueue;

const WRITER_BIT: usize = 1 << (usize::BITS - 1);

pub struct RwLock<T> {
    state: AtomicUsize,
    writer_queue: TicketLock<WaitQueue>,
    reader_queue: TicketLock<WaitQueue>,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for RwLock<T> {}
unsafe impl<T: Send> Send for RwLock<T> {}

pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> RwLock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            state: AtomicUsize::new(0),
            writer_queue: TicketLock::new(WaitQueue::new()),
            reader_queue: TicketLock::new(WaitQueue::new()),
            data: UnsafeCell::new(data),
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        loop {
            let current = self.state.load(Ordering::Acquire);

            if (current & WRITER_BIT) == 0 {
                if self.state.compare_exchange_weak(current, current + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    return RwLockReadGuard { lock: self };
                }
            } else {
                let int_state = interrupts_enabled();
                disable_interrupts();

                let mut rq = self.reader_queue.lock();

                if (self.state.load(Ordering::Acquire) & WRITER_BIT) != 0 {
                    let thread = get_core_data().scheduler.get_current_thread();
                    unsafe { (*thread).state = ThreadState::Blocked };
                    rq.push(thread);
                    drop(rq);

                    get_core_data().scheduler.schedule();
                    if int_state {
                        enable_interrupts()
                    };
                } else {
                    drop(rq);
                    if int_state {
                        enable_interrupts()
                    };
                }
            }
        }
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        loop {
            let current = self.state.load(Ordering::Acquire);

            if current == 0 {
                if self.state.compare_exchange_weak(0, WRITER_BIT, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    return RwLockWriteGuard { lock: self };
                }
            } else {
                let int_state = interrupts_enabled();
                disable_interrupts();

                let mut wq = self.writer_queue.lock();

                if self.state.load(Ordering::Acquire) != 0 {
                    let thread = get_core_data().scheduler.get_current_thread();
                    unsafe { (*thread).state = ThreadState::Blocked };
                    wq.push(thread);
                    drop(wq);

                    get_core_data().scheduler.schedule();
                    if int_state {
                        enable_interrupts()
                    };
                } else {
                    drop(wq);
                    if int_state {
                        enable_interrupts()
                    };
                }
            }
        }
    }
}

impl<'a, T> Deref for RwLockReadGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { unsafe { &*self.lock.data.get() } }
}

impl<'a, T> Deref for RwLockWriteGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { unsafe { &*self.lock.data.get() } }
}

impl<'a, T> DerefMut for RwLockWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { unsafe { &mut *self.lock.data.get() } }
}

impl<'a, T> Drop for RwLockReadGuard<'a, T> {
    fn drop(&mut self) {
        let prev = self.lock.state.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            let int_state = interrupts_enabled();
            disable_interrupts();

            let mut wq = self.lock.writer_queue.lock();
            let thread = wq.pop();
            drop(wq);

            if !thread.is_null() {
                wake_thread(thread);
            }
            if int_state {
                enable_interrupts()
            };
        }
    }
}

impl<'a, T> Drop for RwLockWriteGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);

        let int_state = interrupts_enabled();
        disable_interrupts();

        let mut wq = self.lock.writer_queue.lock();
        let wthread = wq.pop();
        drop(wq);

        if !wthread.is_null() {
            wake_thread(wthread);
        } else {
            let mut rq = self.lock.reader_queue.lock();
            loop {
                let rthread = rq.pop();
                if rthread.is_null() {
                    break;
                }
                wake_thread(rthread);
            }
            drop(rq);
        }
        if int_state {
            enable_interrupts()
        };
    }
}

impl<T: Debug> Debug for RwLock<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("RwLock");

        let current_state = self.state.load(Ordering::Relaxed);
        if current_state & WRITER_BIT != 0 {
            d.field("data", &"<locked>");
        } else {
            unsafe {
                d.field("data", &*self.data.get());
            }
        }
        d.finish()
    }
}
