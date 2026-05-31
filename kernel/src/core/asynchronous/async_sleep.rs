use core::pin::Pin;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};
use core::task::{
    Context,
    Poll,
};

use crate::arch::get_core_data;
use crate::core::time::callout::{
    Callout,
    CalloutPayload,
};
use crate::core::time::{
    get_time,
    ns_to_ticks,
    update_hardware_timer,
};

pub struct AsyncSleep {
    target_ticks: usize,
    callout_armed: AtomicBool,
}

impl Future for AsyncSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let current_time = get_time();
        if current_time >= self.target_ticks {
            Poll::Ready(())
        } else {
            if !self.callout_armed.swap(true, Ordering::Relaxed) {
                let callout = Callout { wake_time: self.target_ticks, payload: CalloutPayload::WakeWaker(cx.waker().clone()) };

                get_core_data().callout_queue.lock().push(callout);
                update_hardware_timer();
            }
            Poll::Pending
        }
    }
}

pub fn sleep_async(ms: usize) -> AsyncSleep {
    let ticks = ns_to_ticks(ms * 1_000_000);
    AsyncSleep { target_ticks: get_time() + ticks, callout_armed: AtomicBool::new(false) }
}
