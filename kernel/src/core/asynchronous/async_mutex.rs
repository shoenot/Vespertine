use core::{cell::UnsafeCell, ops::{Deref, DerefMut}, task::{Poll, Waker}};

use alloc::collections::vec_deque::VecDeque;
use vespertine_common::lock::TicketLock;


#[derive(Debug)]
pub struct AsyncMutex<T> {
    inner: TicketLock<AsyncMutexInner<T>>,
}

#[derive(Debug)]
struct AsyncMutexInner<T> {
    locked: bool,
    waiters: VecDeque<Waker>,
    data: UnsafeCell<T>,
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
        Self { 
            inner: TicketLock::new(AsyncMutexInner { 
                locked: false,
                waiters: VecDeque::new(),
                data: UnsafeCell::new(value),
            }) 
        }
    }

    pub fn lock(&self) -> AsyncMutexLockFuture<'_, T> {
        AsyncMutexLockFuture { mutex: self }
    }

    pub fn try_lock(&self) -> Option<AsyncMutexGuard<'_,T>> {
        let mut inner = self.inner.lock();
        if !inner.locked {
            inner.locked = true;
            Some(AsyncMutexGuard { mutex: self })
        } else {
            None
        }
    }

    pub(crate) fn unlock(&self) {
        let mut inner = self.inner.lock();
        if let Some(waker) = inner.waiters.pop_front() {
            waker.wake();
        } else {
            inner.locked = false;
        }
    }
}

pub struct AsyncMutexLockFuture<'a, T> {
    mutex: &'a AsyncMutex<T>,
}

impl<'a, T> Future for AsyncMutexLockFuture<'a, T> {
    type Output = AsyncMutexGuard<'a, T>;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
        let mut inner = self.mutex.inner.lock();
        if !inner.locked {
            inner.locked = true;
            Poll::Ready(AsyncMutexGuard { mutex: self.mutex })
        } else {    
            let waker = cx.waker().clone();
            if !inner.waiters.iter().any(|w| w.will_wake(&waker)) {
                inner.waiters.push_back(waker);
            }
            Poll::Pending
        }
    }
}


impl<'a, T> Deref for AsyncMutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.inner.lock().data.get() }
    }
}

impl<'a, T> DerefMut for AsyncMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.inner.lock().data.get() }
    }
}

impl<'a, T> Drop for AsyncMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}
