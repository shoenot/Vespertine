use alloc::sync::{
    Arc,
    Weak,
};
use alloc::vec::Vec;
use core::future::poll_fn;
use core::pin::pin;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};
use core::task::{Poll, Waker};

use crate::sync::TicketLock;

#[derive(Debug)]
pub struct AsyncWaiter {
    active: AtomicBool,
    waker: TicketLock<Option<Waker>>,
}

impl AsyncWaiter {
    pub fn new() -> Arc<Self> { Arc::new(Self { active: AtomicBool::new(true), waker: TicketLock::new(None) }) }

    pub fn register(&self, waker: &Waker) {
        self.active.store(true, Ordering::Release);

        let mut stored = self.waker.lock();
        if stored.as_ref().is_none_or(|old| !old.will_wake(waker)) {
            *stored = Some(waker.clone());
        }
    }

    pub fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
        self.waker.lock().take();
    }

    pub fn wake(&self) {
        let waker = self.waker.lock().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn take_waker(&self) -> Option<Waker> {
        if !self.active.load(Ordering::Acquire) {
            return None;
        }

        self.waker.lock().take()
    }
}

#[derive(Debug)]
pub struct TerminationToken {
    requested: AtomicBool,
    waiters: TicketLock<WaiterList>,
}

impl TerminationToken {
    pub fn new() -> Self {
        Self { requested: AtomicBool::new(false), waiters: TicketLock::new(WaiterList::new()) }
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    // returns false if termination was already requested
    pub fn register(&self, waiter: &Arc<AsyncWaiter>, waker: &Waker) -> bool {
        let mut waiters = self.waiters.lock();
        if self.is_requested() { return false };
        waiters.register(waiter, waker);
        true
    }

    pub fn request_and_take_wakers(&self) -> Vec<Waker> {
        let mut waiters = self.waiters.lock();
        if self.requested.swap(true, Ordering::AcqRel) { return Vec::new() };
        waiters.take_wakers()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terminating;

pub async fn interruptible_wait<F>(token: &TerminationToken, future: F) 
    -> Result<F::Output, Terminating> where F: Future {
        let mut future = pin!(future);
        let mut cancellation_waiter: Option<Arc<AsyncWaiter>> = None;

    poll_fn(|cx| {
        if token.is_requested() {
            return Poll::Ready(Err(Terminating));
        }

        if let Poll::Ready(output) = future.as_mut().poll(cx) {
            return Poll::Ready(Ok(output));
        }

        let waiter = cancellation_waiter.get_or_insert_with(AsyncWaiter::new);
        if token.register(waiter, cx.waker()) {
            Poll::Pending
        } else {
            Poll::Ready(Err(Terminating))
        }
    })
    .await
}

#[derive(Debug)]
pub struct WaiterList {
    waiters: Vec<Weak<AsyncWaiter>>,
}

impl WaiterList {
    pub const fn new() -> Self { Self { waiters: Vec::new() } }

    pub fn register(&mut self, waiter: &Arc<AsyncWaiter>, waker: &Waker) {
        waiter.register(waker);

        let mut already_registered = false;
        self.waiters.retain(|entry| {
            let Some(existing) = entry.upgrade() else {
                return false;
            };
            already_registered |= Arc::ptr_eq(&existing, waiter);
            true
        });

        if !already_registered {
            self.waiters.push(Arc::downgrade(waiter));
        }
    }

    pub fn take_wakers(&mut self) -> Vec<Waker> {
        let mut wakers = Vec::new();

        self.waiters.retain(|weak| {
            let Some(waiter) = weak.upgrade() else {
                return false;
            };

            if !waiter.active.load(Ordering::Acquire) {
                return false;
            }

            if let Some(waker) = waiter.take_waker() {
                wakers.push(waker);
            }

            true
        });

        wakers
    }
}

pub fn wake_all(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}
