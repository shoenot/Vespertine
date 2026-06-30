use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};
use core::task::Waker;

use hal::interrupts::{
    disable_interrupts,
    enable_interrupts,
};

use crate::KERNEL_PROCESS;
use crate::cpu::{KernelCoreData, current_core_id, current_core_mut};
use crate::sync::TicketLock;
use crate::sched::ThreadState;
use crate::sched::block::ThreadWakeRegistration;
use crate::sched::dispatch::{cancel_block_if_awoken, create_tcb};
use crate::sched::priority::ThreadPriority;
use crate::time::get_time;

pub struct TimerRegistration {
    active: AtomicBool,
    fired: AtomicBool,
    waker: TicketLock<Option<Waker>>,
}

impl TimerRegistration {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { active: AtomicBool::new(true), fired: AtomicBool::new(false), waker: TicketLock::new(None) })
    }

    pub fn register(&self, waker: &Waker) {
        let mut stored = self.waker.lock();

        if !self.active.load(Ordering::Acquire) {
            return;
        }

        if stored.as_ref().is_none_or(|old| !old.will_wake(waker)) {
            *stored = Some(waker.clone());
        }
    }

    pub fn cancel(&self) {
        let mut stored = self.waker.lock();
        self.active.store(false, Ordering::Release);
        stored.take();
    }

    pub fn is_fired(&self) -> bool { self.fired.load(Ordering::Acquire) }

    fn fire(&self) -> Option<Waker> {
        if !self.active.swap(false, Ordering::AcqRel) {
            return None;
        }

        self.fired.store(true, Ordering::Release);
        self.waker.lock().take()
    }
}

pub enum CalloutPayload {
    /// used by sleep(), contains pointer to sleeping thread
    WakeThread(Arc<ThreadWakeRegistration>),
    /// used by sleep_async(), contains the async timer registration
    WakeTimer(Arc<TimerRegistration>),
}

pub struct Callout {
    pub wake_time: usize,
    pub payload: CalloutPayload,
}

// Flip the cmp logic backwards bc we want the earliest callout to rise to the top

impl PartialEq for Callout {
    fn eq(&self, other: &Self) -> bool { self.wake_time == other.wake_time }
}

impl Eq for Callout {}

impl Ord for Callout {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering { other.wake_time.cmp(&self.wake_time) }
}

impl PartialOrd for Callout {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> { Some(self.cmp(other)) }
}

unsafe impl Send for Callout {}

pub fn dispatch_callout_payload(payload: CalloutPayload) {
    match payload {
        CalloutPayload::WakeThread(registration) => {
            registration.wake();
        }
        CalloutPayload::WakeTimer(registration) => {
            if let Some(waker) = registration.fire() {
                waker.wake();
            }
        }
    }
}

pub extern "C" fn timer_daemon(_arg: usize) -> ! {
    loop {
        current_core_mut().timer_daemon_awoken.store(false, core::sync::atomic::Ordering::Release);
        disable_interrupts();

        loop {
            let mut queue = current_core_mut().callout_queue.lock();
            let current_time = get_time();

            if let Some(earliest) = queue.peek() {
                if earliest.wake_time <= current_time {
                    if queue.len() == 0 {}
                    let expired = queue.pop().unwrap();
                    drop(queue);

                    dispatch_callout_payload(expired.payload);
                    continue;
                }
            }
            drop(queue);
            break;
        }

        let thread = unsafe { &*current_core_mut().scheduler.current_thread };
        thread.transition(ThreadState::Running, ThreadState::Blocked).expect("timer daemon was not running");

        if cancel_block_if_awoken(thread, &current_core_mut().timer_daemon_awoken) {
            enable_interrupts();
            continue;
        }

        current_core_mut().scheduler.schedule(crate::sched::scheduler::ScheduleReason::Blocked);
        enable_interrupts();
    }
}

pub fn init_timer_daemon(data_ptr: *mut KernelCoreData) {
    assert!(!data_ptr.is_null(), "timer daemon core data was null");
    unsafe {
        let core_data = &mut *data_ptr;
        let timer_daemon_tcb = create_tcb(timer_daemon as *const () as usize, 0, ThreadPriority::HIGH, KERNEL_PROCESS.clone()).unwrap();
        (*timer_daemon_tcb).pin_to_core(core_data.logical_id);
        core_data.timer_daemon_tcb = timer_daemon_tcb;
        core_data.scheduler.push(timer_daemon_tcb);
    }
}

