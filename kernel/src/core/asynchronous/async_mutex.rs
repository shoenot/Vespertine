use alloc::collections::vec_deque::VecDeque;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::ops::{
    Deref,
    DerefMut,
};
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};
use core::task::{
    Poll,
    Waker,
};

use crate::core::sync::TicketLock;

#[derive(Debug)]
pub struct AsyncMutex<T> {
    inner: TicketLock<AsyncMutexInner<T>>,
}

#[derive(Debug)]
struct AsyncMutexInner<T> {
    locked: bool,
    waiters: VecDeque<Arc<Waiter>>,
    data: UnsafeCell<T>,
}

#[derive(Debug)]
struct Waiter {
    active: AtomicBool,
    waker: TicketLock<Option<Waker>>,
}

impl Waiter {
    fn new() -> Self { Self { active: AtomicBool::new(true), waker: TicketLock::new(None) } }
}

unsafe impl<T: Send> Send for AsyncMutex<T> {}
unsafe impl<T: Send> Sync for AsyncMutex<T> {}

pub struct AsyncMutexGuard<'a, T> {
    mutex: &'a AsyncMutex<T>,
}

unsafe impl<T: Send> Send for AsyncMutexGuard<'_, T> {}
unsafe impl<T: Sync> Sync for AsyncMutexGuard<'_, T> {}

impl<T> AsyncMutex<T> {
    pub const fn new(value: T) -> Self {
        Self { inner: TicketLock::new(AsyncMutexInner { locked: false, waiters: VecDeque::new(), data: UnsafeCell::new(value) }) }
    }

    pub fn lock(&self) -> AsyncMutexLockFuture<'_, T> { AsyncMutexLockFuture { mutex: self, waiter: Arc::new(Waiter::new()) } }

    pub fn try_lock(&self) -> Option<AsyncMutexGuard<'_, T>> {
        let mut inner = self.inner.lock();
        if !inner.locked {
            inner.locked = true;
            Some(AsyncMutexGuard { mutex: self })
        } else {
            None
        }
    }

    pub(crate) fn unlock(&self) {
        let waiter = {
            let mut inner = self.inner.lock();
            inner.locked = false;
            loop {
                match inner.waiters.pop_front() {
                    Some(waiter) if waiter.active.load(Ordering::Acquire) => break Some(waiter),
                    Some(_) => continue,
                    None => break None,
                }
            }
        };

        if let Some(waiter) = waiter {
            if waiter.active.swap(false, Ordering::AcqRel) {
                let waker = waiter.waker.lock().take();
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        }
    }
}

pub struct AsyncMutexLockFuture<'a, T> {
    mutex: &'a AsyncMutex<T>,
    waiter: Arc<Waiter>,
}

impl<'a, T> Future for AsyncMutexLockFuture<'a, T> {
    type Output = AsyncMutexGuard<'a, T>;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        let mut inner = self.mutex.inner.lock();
        if !inner.locked {
            inner.locked = true;
            self.waiter.active.store(false, Ordering::Release);
            Poll::Ready(AsyncMutexGuard { mutex: self.mutex })
        } else {
            self.waiter.active.store(true, Ordering::Release);
            *self.waiter.waker.lock() = Some(cx.waker().clone());
            if !inner.waiters.iter().any(|waiter| Arc::ptr_eq(waiter, &self.waiter)) {
                inner.waiters.push_back(self.waiter.clone());
            }
            Poll::Pending
        }
    }
}

impl<T> Drop for AsyncMutexLockFuture<'_, T> {
    fn drop(&mut self) { self.waiter.active.store(false, Ordering::Release); }
}

impl<'a, T> Deref for AsyncMutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { unsafe { &*self.mutex.inner.lock().data.get() } }
}

impl<'a, T> DerefMut for AsyncMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target { unsafe { &mut *self.mutex.inner.lock().data.get() } }
}

impl<'a, T> Drop for AsyncMutexGuard<'a, T> {
    fn drop(&mut self) { self.mutex.unlock(); }
}
