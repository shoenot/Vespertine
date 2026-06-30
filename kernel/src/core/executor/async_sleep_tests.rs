use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::pin::Pin;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};
use core::task::{
    Context,
    Poll,
};

use super::AsyncSleep;
use crate::core::time::callout::{
    CalloutPayload,
    TimerRegistration,
    dispatch_callout_payload,
};

struct CountingWaker {
    wakes: AtomicUsize,
}

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) { self.wake_by_ref(); }

    fn wake_by_ref(self: &Arc<Self>) { self.wakes.fetch_add(1, Ordering::Relaxed); }
}

fn context() -> (Arc<CountingWaker>, core::task::Waker) {
    let counter = Arc::new(CountingWaker { wakes: AtomicUsize::new(0) });
    let waker = counter.clone().into();
    (counter, waker)
}

fn poll_sleep(sleep: &mut AsyncSleep, context: &mut Context<'_>) -> Poll<()> { Pin::new(sleep).poll(context) }

fn armed_sleep() -> AsyncSleep { AsyncSleep { target_ticks: usize::MAX, armed: true, registration: TimerRegistration::new() } }

fn test_dropping_armed_sleep_prevents_wake() {
    let mut sleep = armed_sleep();
    let registration = sleep.registration();
    let (counter, waker) = context();
    assert!(poll_sleep(&mut sleep, &mut Context::from_waker(&waker)).is_pending());

    drop(sleep);
    dispatch_callout_payload(CalloutPayload::WakeTimer(registration));
    assert_eq!(counter.wakes.load(Ordering::Acquire), 0);
}

fn test_replacing_sleep_cancels_old_registration() {
    let mut original = armed_sleep();
    let original_registration = original.registration();
    let (counter, waker) = context();
    assert!(poll_sleep(&mut original, &mut Context::from_waker(&waker)).is_pending());

    let mut slot = Box::pin(original);
    let old = core::mem::replace(&mut slot, Box::pin(armed_sleep()));
    drop(old);

    dispatch_callout_payload(CalloutPayload::WakeTimer(original_registration));
    assert_eq!(counter.wakes.load(Ordering::Acquire), 0);

    drop(slot);
}

fn test_polling_with_new_waker_replaces_stored_waker() {
    let mut sleep = armed_sleep();
    let registration = sleep.registration();
    let (counter1, waker1) = context();
    let (counter2, waker2) = context();

    assert!(poll_sleep(&mut sleep, &mut Context::from_waker(&waker1)).is_pending());
    assert!(poll_sleep(&mut sleep, &mut Context::from_waker(&waker2)).is_pending());

    dispatch_callout_payload(CalloutPayload::WakeTimer(registration));
    assert_eq!(counter1.wakes.load(Ordering::Acquire), 0);
    assert_eq!(counter2.wakes.load(Ordering::Acquire), 1);
}

fn test_firing_timer_wakes_once() {
    let mut sleep = armed_sleep();
    let registration = sleep.registration();
    let (counter, waker) = context();
    assert!(poll_sleep(&mut sleep, &mut Context::from_waker(&waker)).is_pending());

    dispatch_callout_payload(CalloutPayload::WakeTimer(registration.clone()));
    dispatch_callout_payload(CalloutPayload::WakeTimer(registration));
    assert_eq!(counter.wakes.load(Ordering::Acquire), 1);
}

fn test_cancelling_after_firing_does_not_wake_again() {
    let mut sleep = armed_sleep();
    let registration = sleep.registration();
    let (counter, waker) = context();
    assert!(poll_sleep(&mut sleep, &mut Context::from_waker(&waker)).is_pending());

    dispatch_callout_payload(CalloutPayload::WakeTimer(registration.clone()));
    registration.cancel();
    dispatch_callout_payload(CalloutPayload::WakeTimer(registration));
    assert_eq!(counter.wakes.load(Ordering::Acquire), 1);
}

pub(super) fn run() {
    crate::klogln!("[TEST] async sleep drop cancels");
    test_dropping_armed_sleep_prevents_wake();
    crate::klogln!("[TEST] async sleep replacement cancels");
    test_replacing_sleep_cancels_old_registration();
    crate::klogln!("[TEST] async sleep waker update");
    test_polling_with_new_waker_replaces_stored_waker();
    crate::klogln!("[TEST] async sleep fires once");
    test_firing_timer_wakes_once();
    crate::klogln!("[TEST] async sleep cancel after fire");
    test_cancelling_after_firing_does_not_wake_again();
}
