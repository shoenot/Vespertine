use alloc::sync::Arc;
use core::pin::Pin;
use core::task::{
    Context,
    Poll,
};

use crate::cpu::current_core_mut;
use crate::core::time::callout::{
    Callout,
    CalloutPayload,
    TimerRegistration,
};
use crate::core::time::{
    get_time,
    ns_to_ticks,
    update_hardware_timer,
};

pub struct AsyncSleep {
    target_ticks: usize,
    armed: bool,
    registration: Arc<TimerRegistration>,
}

impl Future for AsyncSleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.registration.is_fired() || get_time() >= self.target_ticks {
            self.registration.cancel();
            return Poll::Ready(());
        }

        self.registration.register(cx.waker());

        if !self.armed {
            self.armed = true;

            // AsyncSleep callouts are armed on the polling CPU.
            // Futures using AsyncSleep must not migrate between executors.
            let callout = Callout { wake_time: self.target_ticks, payload: CalloutPayload::WakeTimer(self.registration.clone()) };

            current_core_mut().callout_queue.lock().push(callout);
            update_hardware_timer();
        }

        Poll::Pending
    }
}

pub fn sleep_async(ms: usize) -> AsyncSleep {
    let ticks = ns_to_ticks(ms.saturating_mul(1_000_000));

    AsyncSleep { target_ticks: get_time().saturating_add(ticks), armed: false, registration: TimerRegistration::new() }
}

impl Drop for AsyncSleep {
    fn drop(&mut self) { self.registration.cancel(); }
}

impl AsyncSleep {
    pub(crate) fn registration(&self) -> Arc<TimerRegistration> { self.registration.clone() }
}

#[path = "async_sleep_tests.rs"]
mod tests;

pub(super) fn run_diagnostic_tests() { tests::run(); }
