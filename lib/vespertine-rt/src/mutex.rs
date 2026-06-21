use core::cell::UnsafeCell;
use core::ops::{
    Deref,
    DerefMut,
};
use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use crate::syscall::{
    sys_futex_wait,
    sys_futex_wake,
};

// 0 - unlocked
// 1 - locked (no waiters)
// 2 - locked (with waiters)

pub struct Mutex<T> {
    locked: AtomicU32,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

pub struct MutexGuard<'a, T> {
    lock: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    pub const fn new(val: T) -> Self { Self { locked: AtomicU32::new(0), data: UnsafeCell::new(val) } }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        // fast path = if 0 change to 1
        while self.locked.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return MutexGuard { lock: self };
        }

        // slow path = wq
        loop {
            // attempt to grab and set it to 2 (showing waiters exist)
            if self.locked.swap(2, Ordering::Acquire) == 0 {
                return MutexGuard { lock: self };
            }

            // sleep until no longer 2
            let addr = &self.locked as *const AtomicU32 as usize;
            sys_futex_wait(addr, 2);
        }
    }

    // use only for panics plz
    pub unsafe fn force_unlock(&self) {
        self.locked.store(0, Ordering::Release);
        sys_futex_wake(&self.locked as *const AtomicU32 as usize, 1);
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.lock.data.get() } }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.lock.data.get() } }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        // if 2 (waiters exist) we need to wake someone up
        // if 1, just set it to 0
        if self.lock.locked.swap(0, Ordering::Release) == 2 {
            let addr = &self.lock.locked as *const AtomicU32 as usize;
            sys_futex_wake(addr, 1);
        }
    }
}
