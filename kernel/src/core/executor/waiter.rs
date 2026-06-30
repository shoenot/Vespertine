use alloc::sync::{
    Arc,
    Weak,
};
use alloc::vec::Vec;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};
use core::task::Waker;

use crate::core::sync::TicketLock;

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
pub struct WaiterList {
    waiters: Vec<Weak<AsyncWaiter>>,
}

impl WaiterList {
    pub const fn new() -> Self { Self { waiters: Vec::new() } }

    pub fn register(&mut self, waiter: &Arc<AsyncWaiter>, waker: &Waker) {
        waiter.register(waker);

        let already_registered =
            self.waiters.iter().any(|existing| existing.upgrade().is_some_and(|existing| Arc::ptr_eq(&existing, waiter)));

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
