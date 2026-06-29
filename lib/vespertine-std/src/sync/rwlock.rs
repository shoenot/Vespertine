use core::cell::UnsafeCell;
use core::ops::{
    Deref,
    DerefMut,
};
use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use vespertine_rt::syscall::{
    sys_futex_wait,
    sys_futex_wake,
};

const WRITER_BIT: u32 = 1 << 31;
const READER_MASK: u32 = !WRITER_BIT;

pub struct RwLock<T> {
    state: AtomicU32,
    writers_waiting: AtomicU32,
    data: UnsafeCell<T>,
}

pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    pub const fn new(data: T) -> Self { Self { state: AtomicU32::new(0), writers_waiting: AtomicU32::new(0), data: UnsafeCell::new(data) } }

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        let addr = &self.state as *const AtomicU32 as usize;

        loop {
            let state = self.state.load(Ordering::Acquire);

            if state & WRITER_BIT != 0 || self.writers_waiting.load(Ordering::Acquire) != 0 {
                sys_futex_wait(addr, state);
                continue;
            }

            if state & READER_MASK == READER_MASK {
                sys_futex_wait(addr, state);
                continue;
            }

            if self.state.compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return RwLockReadGuard { lock: self };
            }
        }
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        let addr = &self.state as *const AtomicU32 as usize;

        self.writers_waiting.fetch_add(1, Ordering::AcqRel);

        loop {
            let state = self.state.load(Ordering::Acquire);

            if state == 0 && self.state.compare_exchange_weak(0, WRITER_BIT, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                self.writers_waiting.fetch_sub(1, Ordering::AcqRel);
                return RwLockWriteGuard { lock: self };
            }

            sys_futex_wait(addr, state);
        }
    }

    pub fn replace(&self, value: T) -> T {
        let mut guard = self.write();
        core::mem::replace(&mut *guard, value)
    }

    fn wake(&self, count: usize) {
        let addr = &self.state as *const AtomicU32 as usize;
        sys_futex_wake(addr, count);
    }
}

impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { unsafe { &*self.lock.data.get() } }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        let previous = self.lock.state.fetch_sub(1, Ordering::Release);

        if previous == 1 {
            self.lock.wake(1);
        }
    }
}

impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { unsafe { &*self.lock.data.get() } }
}

impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { unsafe { &mut *self.lock.data.get() } }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);
        self.lock.wake(usize::MAX);
    }
}
