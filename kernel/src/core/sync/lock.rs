use core::cell::UnsafeCell;
use core::ops::{
    Deref,
    DerefMut,
};
use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    interrupts_enabled,
};

use vespertine_common::lock::{RawLock, RawSpinLock, RawTicketLock};

// Kernel version of the generic lock

#[derive(Debug)]
pub struct Lock<R: RawLock, T> {
    raw: R,
    data: UnsafeCell<T>,
}

impl<R: RawLock, T> Lock<R, T> {
    pub fn lock(&self) -> LockGuard<'_, R, T> {
        let interrupts_state = interrupts_enabled();
        if interrupts_state {
            disable_interrupts();
        }
        self.raw.lock();
        LockGuard { lock: self, interrupts_state }
    }

    // ONLY USE DURING KERNEL PANICS PLZ
    pub unsafe fn force_unlock(&self) {
        unsafe {
            self.raw.force_unlock();
        }
    }
}

unsafe impl<R: RawLock + Send, T> Send for Lock<R, T> where T: Send {}
unsafe impl<R: RawLock + Sync, T> Sync for Lock<R, T> where T: Send {}

// lock guard wrapper

#[derive(Debug)]
pub struct LockGuard<'a, R: RawLock, T> {
    lock: &'a Lock<R, T>,
    interrupts_state: bool,
}

unsafe impl<R: RawLock, T> Send for LockGuard<'_, R, T> where T: Send {}
unsafe impl<R: RawLock, T> Sync for LockGuard<'_, R, T> where T: Sync {}

impl<'a, L: RawLock, T> Deref for LockGuard<'_, L, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.lock.data.get() } }
}

impl<'a, R: RawLock, T> DerefMut for LockGuard<'_, R, T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.lock.data.get() } }
}

impl<'a, R: RawLock, T> Drop for LockGuard<'_, R, T> {
    fn drop(&mut self) {
        self.lock.raw.unlock();
        if self.interrupts_state {
            enable_interrupts();
        }
    }
}

// Clean constructors

pub type SpinLock<T> = Lock<RawSpinLock, T>;
pub type TicketLock<T> = Lock<RawTicketLock, T>;

impl<T> SpinLock<T> {
    pub const fn new(val: T) -> Self { Self { raw: RawSpinLock::new(), data: UnsafeCell::new(val) } }
}

impl<T> TicketLock<T> {
    pub const fn new(val: T) -> Self { Self { raw: RawTicketLock::new(), data: UnsafeCell::new(val) } }
}
