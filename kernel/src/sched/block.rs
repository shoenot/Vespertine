use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::sched::Thread;
use crate::sched::dispatch::wake_thread;

#[derive(Debug)]
pub struct ThreadWakeRegistration {
    active: AtomicBool,
    fired: AtomicBool,
    thread: *mut Thread,
}

unsafe impl Send for ThreadWakeRegistration {}
unsafe impl Sync for ThreadWakeRegistration {}

impl ThreadWakeRegistration {
    pub fn new(thread: *mut Thread) -> Arc<Self> {
        Arc::new(Self { active: AtomicBool::new(true), fired: AtomicBool::new(false), thread })
    }

    pub fn cancel(&self) -> bool { self.active.swap(false, Ordering::AcqRel) }

    pub fn wake(&self) -> bool {
        if !self.active.swap(false, Ordering::AcqRel) {
            return false;
        }
        self.fired.store(true, Ordering::Release);
        wake_thread(self.thread);
        true
    }

    pub fn is_active(&self) -> bool { self.active.load(Ordering::Acquire) }

    pub fn fired(&self) -> bool { self.fired.load(Ordering::Acquire) }

    pub fn thread(&self) -> *mut Thread { self.thread }
}
