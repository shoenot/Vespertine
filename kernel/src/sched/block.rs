use alloc::sync::Arc;
use hal::interrupts::{disable_interrupts, enable_interrupts, interrupts_enabled};
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::sched::{Thread, ThreadPin};
use crate::sched::dispatch::wake_thread;
use crate::sync::TicketLock;

#[derive(Debug)]
pub struct ThreadWakeRegistration {
    active: AtomicBool,
    fired: AtomicBool,
    target: Arc<WakeTarget>,
}

#[derive(Debug)]
struct WakeTcb(*mut Thread);

unsafe impl Send for WakeTcb {}

#[derive(Debug)]
pub struct WakeTarget {
    live: TicketLock<Option<WakeTcb>>,
}

impl WakeTarget {
    // install this target on the given tcb before publishing the tcb
    // the reaper must detach the target before freeing the tcb
    pub unsafe fn new(thread: *mut Thread) -> Arc<Self> {
        Arc::new(Self { live: TicketLock::new(Some(WakeTcb(thread))) })
    }

    pub fn detach(&self) {
        *self.live.lock() = None;
    }

    fn try_pin(&self) -> Option<ThreadPin> {
        let live = self.live.lock();
        let thread = live.as_ref()?.0;
        Some(unsafe { ThreadPin::acquire_protected(thread) })
    }
}

impl ThreadWakeRegistration {
    pub fn new(target: Arc<WakeTarget>) -> Arc<Self> {
        Arc::new(Self { active: AtomicBool::new(true), fired: AtomicBool::new(false), target })
    }

    pub fn cancel(&self) -> bool { self.active.swap(false, Ordering::AcqRel) }

    pub fn wake(&self) -> bool {
        let int_state = interrupts_enabled();
        disable_interrupts();

        let accepted = self.wake_irqs_off();

        if int_state { enable_interrupts() };

        accepted
    }

    fn wake_irqs_off(&self) -> bool {
        if !self.active.swap(false, Ordering::AcqRel) { return false };
        let Some(pin) = self.target.try_pin() else { return false };

        self.fired.store(true, Ordering::Release);
        wake_thread(pin.as_ptr());
        true
    }

    pub fn is_active(&self) -> bool { self.active.load(Ordering::Acquire) }

    pub fn fired(&self) -> bool { self.fired.load(Ordering::Acquire) }
}
