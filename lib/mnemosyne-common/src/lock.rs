use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{
    Deref,
    DerefMut,
};
use core::sync::atomic::Ordering::{
    Acquire,
    Relaxed,
    Release,
};
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
};

pub trait RawLock {
    fn lock(&self);
    fn unlock(&self);
    unsafe fn force_unlock(&self);
}

// Raw SpinLock

#[derive(Debug)]
pub struct RawSpinLock {
    locked: AtomicBool,
}

impl RawSpinLock {
    pub const fn new() -> Self { Self { locked: AtomicBool::new(false) } }
}

impl RawLock for RawSpinLock {
    fn lock(&self) {
        loop {
            if !self.locked.swap(true, Acquire) {
                break;
            }

            while self.locked.load(Relaxed) {
                spin_loop();
            }
        }
    }

    fn unlock(&self) { self.locked.store(false, Release) }

    unsafe fn force_unlock(&self) { self.locked.store(false, Release); }
}

// Raw TicketLock

#[derive(Debug)]
pub struct RawTicketLock {
    ticket: AtomicUsize,
    serving: AtomicUsize,
}

impl RawTicketLock {
    pub const fn new() -> Self { Self { ticket: AtomicUsize::new(0), serving: AtomicUsize::new(0) } }
}

impl RawLock for RawTicketLock {
    fn lock(&self) {
        let ticket = self.ticket.fetch_add(1, Relaxed);

        while self.serving.load(Acquire) != ticket {
            spin_loop();
        }
    }

    fn unlock(&self) {
        let successor = self.serving.load(Relaxed) + 1;
        self.serving.store(successor, Release);
    }

    unsafe fn force_unlock(&self) { self.serving.store(self.ticket.load(Relaxed), Release); }
}

unsafe impl Send for RawTicketLock {}
unsafe impl Sync for RawTicketLock {}

// Generic Lock

#[derive(Debug)]
pub struct Lock<R: RawLock, T> {
    raw: R,
    data: UnsafeCell<T>,
}

impl<R: RawLock, T> Lock<R, T> {
    pub fn lock(&self) -> LockGuard<'_, R, T> {
        self.raw.lock();
        LockGuard { lock: self }
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
