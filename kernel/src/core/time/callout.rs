use core::task::Waker;

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
};
use crate::core::thread::dispatch::{
    cancel_block_if_awoken,
    wake_thread,
};
use crate::core::thread::{
    ThreadControlBlock,
    ThreadState,
};
use crate::core::time::get_time;

pub enum CalloutPayload {
    /// used by sleep(), contains pointer to sleeping thread
    WakeThread(*mut ThreadControlBlock),
    /// used by sleep_async(), contains the async task's waker
    WakeWaker(Waker),
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

pub extern "C" fn timer_daemon(_arg: usize) -> ! {
    loop {
        get_core_data().timer_daemon_awoken.store(false, core::sync::atomic::Ordering::Release);
        disable_interrupts();

        loop {
            let mut queue = get_core_data().callout_queue.lock();
            let current_time = get_time();

            if let Some(earliest) = queue.peek() {
                if earliest.wake_time <= current_time {
                    if queue.len() == 0 {}
                    let expired = queue.pop().unwrap();
                    drop(queue);

                    match expired.payload {
                        CalloutPayload::WakeThread(tcb_ptr) => wake_thread(tcb_ptr),
                        CalloutPayload::WakeWaker(waker) => {
                            waker.wake();
                        }
                    }
                    continue;
                }
            }
            drop(queue);
            break;
        }

        let thread = unsafe { &*get_core_data().scheduler.current_thread };
        thread.transition(ThreadState::Running, ThreadState::Blocked).expect("timer daemon was not running");

        if cancel_block_if_awoken(thread, &get_core_data().timer_daemon_awoken) {
            enable_interrupts();
            continue;
        }

        get_core_data().scheduler.schedule();
        enable_interrupts();
    }
}
