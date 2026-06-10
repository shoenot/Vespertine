use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};
use core::task::{
    Context,
    Poll,
};

use vespertine_abi::Signal;

use super::{
    BUFFER_SIZE,
    SocketEndpoint,
    SocketReadFuture,
    SocketWaitFuture,
    SocketWriteFuture,
};
use crate::arch::get_core_data;
use crate::core::asynchronous::waiter::AsyncWaiter;
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

fn poll<F: Future>(future: &mut F, context: &mut Context<'_>) -> Poll<F::Output> { unsafe { Pin::new_unchecked(future) }.poll(context) }

fn remove_timer_registrations(registrations: &[Arc<TimerRegistration>]) {
    let mut queue = get_core_data().callout_queue.lock();
    let mut retained = vec![];

    while let Some(callout) = queue.pop() {
        let keep = match &callout.payload {
            CalloutPayload::WakeTimer(registration) => !registrations.iter().any(|target| Arc::ptr_eq(registration, target)),
            _ => true,
        };

        if keep {
            retained.push(callout);
        }
    }

    for callout in retained {
        queue.push(callout);
    }
}

fn test_two_readers_are_woken() {
    let (reader, writer) = SocketEndpoint::new_pair();
    let mut out1 = [0u8; 1];
    let mut out2 = [0u8; 1];
    let mut read1 = SocketReadFuture::new(&reader, out1.as_mut_ptr() as usize, 1, 1, 0);
    let mut read2 = SocketReadFuture::new(&reader, out2.as_mut_ptr() as usize, 1, 1, 0);
    let (counter1, waker1) = context();
    let (counter2, waker2) = context();
    assert!(poll(&mut read1, &mut Context::from_waker(&waker1)).is_pending());
    assert!(poll(&mut read2, &mut Context::from_waker(&waker2)).is_pending());

    let input = [7u8];
    let mut write = SocketWriteFuture::new(&writer, input.as_ptr() as usize, input.len());
    let (_, write_waker) = context();
    assert!(poll(&mut write, &mut Context::from_waker(&write_waker)).is_ready());

    assert_eq!(counter1.wakes.load(Ordering::Acquire), 1);
    assert_eq!(counter2.wakes.load(Ordering::Acquire), 1);
}

fn test_two_writers_are_woken() {
    let (reader, writer) = SocketEndpoint::new_pair();
    {
        let mut inner = writer.write_bus.inner.lock();
        inner.buffer.push_slice(&vec![1u8; BUFFER_SIZE - 1]);
    }

    let input1 = [2u8];
    let input2 = [3u8];
    let mut write1 = SocketWriteFuture::new(&writer, input1.as_ptr() as usize, 1);
    let mut write2 = SocketWriteFuture::new(&writer, input2.as_ptr() as usize, 1);
    let (counter1, waker1) = context();
    let (counter2, waker2) = context();
    assert!(poll(&mut write1, &mut Context::from_waker(&waker1)).is_pending());
    assert!(poll(&mut write2, &mut Context::from_waker(&waker2)).is_pending());

    let mut output = [0u8; 1];
    let mut read = SocketReadFuture::new(&reader, output.as_mut_ptr() as usize, 1, 1, 0);
    let (_, read_waker) = context();
    assert!(poll(&mut read, &mut Context::from_waker(&read_waker)).is_ready());

    assert_eq!(counter1.wakes.load(Ordering::Acquire), 1);
    assert_eq!(counter2.wakes.load(Ordering::Acquire), 1);
}

fn test_dropped_reader_is_not_woken() {
    let (reader, writer) = SocketEndpoint::new_pair();
    let mut out1 = [0u8; 1];
    let mut out2 = [0u8; 1];
    let mut dropped = SocketReadFuture::new(&reader, out1.as_mut_ptr() as usize, 1, 1, 0);
    let mut remaining = SocketReadFuture::new(&reader, out2.as_mut_ptr() as usize, 1, 1, 0);
    let (dropped_counter, dropped_waker) = context();
    let (remaining_counter, remaining_waker) = context();
    assert!(poll(&mut dropped, &mut Context::from_waker(&dropped_waker)).is_pending());
    assert!(poll(&mut remaining, &mut Context::from_waker(&remaining_waker)).is_pending());
    drop(dropped);

    writer.write_bus.inner.lock().buffer.push_slice(&[1]);
    writer.write_bus.notify_readable();
    assert_eq!(dropped_counter.wakes.load(Ordering::Acquire), 0);
    assert_eq!(remaining_counter.wakes.load(Ordering::Acquire), 1);
}

fn test_endpoint_close_wakes_peer_waiters() {
    let (closing, peer) = SocketEndpoint::new_pair();
    let mut output = [0u8; 1];
    let mut read = SocketReadFuture::new(&peer, output.as_mut_ptr() as usize, 1, 1, 0);
    let mut wait = SocketWaitFuture { endpoint: &peer, requested: Signal::PEER_CLOSED, waiter: AsyncWaiter::new() };
    let (read_counter, read_waker) = context();
    let (wait_counter, wait_waker) = context();
    assert!(poll(&mut read, &mut Context::from_waker(&read_waker)).is_pending());
    assert!(poll(&mut wait, &mut Context::from_waker(&wait_waker)).is_pending());

    drop(closing);
    assert_eq!(read_counter.wakes.load(Ordering::Acquire), 1);
    assert_eq!(wait_counter.wakes.load(Ordering::Acquire), 1);
}

fn test_read_and_signal_waiters_coexist() {
    let (reader, writer) = SocketEndpoint::new_pair();
    let mut output = [0u8; 1];
    let mut read = SocketReadFuture::new(&reader, output.as_mut_ptr() as usize, 1, 1, 0);
    let mut wait = SocketWaitFuture { endpoint: &reader, requested: Signal::READABLE, waiter: AsyncWaiter::new() };
    let (read_counter, read_waker) = context();
    let (wait_counter, wait_waker) = context();
    assert!(poll(&mut read, &mut Context::from_waker(&read_waker)).is_pending());
    assert!(poll(&mut wait, &mut Context::from_waker(&wait_waker)).is_pending());

    writer.write_bus.inner.lock().buffer.push_slice(&[1]);
    writer.write_bus.notify_readable();
    assert_eq!(read_counter.wakes.load(Ordering::Acquire), 1);
    assert_eq!(wait_counter.wakes.load(Ordering::Acquire), 1);
}

fn test_shared_waiter_across_buses_wakes_once() {
    let (endpoint, _) = SocketEndpoint::new_pair();
    let waiter = AsyncWaiter::new();
    let (counter, waker) = context();
    endpoint.read_bus.inner.lock().readable_signal_waiters.register(&waiter, &waker);
    endpoint.write_bus.inner.lock().writable_signal_waiters.register(&waiter, &waker);

    endpoint.read_bus.notify_readable();
    endpoint.write_bus.notify_writable();
    assert_eq!(counter.wakes.load(Ordering::Acquire), 1);
}

fn test_readable_change_does_not_wake_blocked_writer() {
    let (reader, writer) = SocketEndpoint::new_pair();
    let read_waiter = AsyncWaiter::new();
    let write_waiter = AsyncWaiter::new();
    let (read_counter, read_waker) = context();
    let (write_counter, write_waker) = context();

    writer.write_bus.inner.lock().read_waiters.register(&read_waiter, &read_waker);
    writer.write_bus.inner.lock().write_waiters.register(&write_waiter, &write_waker);
    writer.write_bus.notify_readable();

    assert_eq!(read_counter.wakes.load(Ordering::Acquire), 1);
    assert_eq!(write_counter.wakes.load(Ordering::Acquire), 0);
    drop(reader);
}

fn test_wait_future_rechecks_readiness() {
    let (reader, writer) = SocketEndpoint::new_pair();
    let mut wait = SocketWaitFuture { endpoint: &reader, requested: Signal::READABLE, waiter: AsyncWaiter::new() };
    let (_, waker) = context();
    let mut context = Context::from_waker(&waker);
    let result = wait.poll_with_registration_hook(&mut context, || {
        writer.write_bus.inner.lock().buffer.push_slice(&[1]);
    });
    assert!(result.is_ready());
}

fn test_timed_read_restarts_timer_without_stale_wake() {
    let (reader, writer) = SocketEndpoint::new_pair();
    writer.write_bus.inner.lock().buffer.push_slice(&[1]);

    let mut output = [0u8; 3];
    let mut read = SocketReadFuture::new(&reader, output.as_mut_ptr() as usize, output.len(), 3, 1);
    let (counter, waker) = context();
    let mut context = Context::from_waker(&waker);

    assert!(poll(&mut read, &mut context).is_pending());
    let old_registration = read.timer.as_ref().expect("initial timer missing").registration();

    writer.write_bus.inner.lock().buffer.push_slice(&[2]);
    assert!(poll(&mut read, &mut context).is_pending());
    let new_registration = read.timer.as_ref().expect("replacement timer missing").registration();
    assert!(!Arc::ptr_eq(&old_registration, &new_registration), "timer was not replaced");

    dispatch_callout_payload(CalloutPayload::WakeTimer(old_registration.clone()));
    assert_eq!(counter.wakes.load(Ordering::Acquire), 0, "stale timer wake escaped cancellation");

    dispatch_callout_payload(CalloutPayload::WakeTimer(new_registration.clone()));
    assert_eq!(counter.wakes.load(Ordering::Acquire), 1, "replacement timer did not wake");

    assert_eq!(poll(&mut read, &mut context), Poll::Ready(Ok(2)));
    assert_eq!(&output[..2], &[1, 2]);

    remove_timer_registrations(&[old_registration, new_registration]);
}

pub(super) fn run() {
    crate::klogln!("[TEST] socket multiple readers");
    test_two_readers_are_woken();
    crate::klogln!("[TEST] socket multiple writers");
    test_two_writers_are_woken();
    crate::klogln!("[TEST] socket cancelled reader");
    test_dropped_reader_is_not_woken();
    crate::klogln!("[TEST] socket endpoint closure");
    test_endpoint_close_wakes_peer_waiters();
    crate::klogln!("[TEST] socket mixed waiters");
    test_read_and_signal_waiters_coexist();
    crate::klogln!("[TEST] socket shared waiter");
    test_shared_waiter_across_buses_wakes_once();
    crate::klogln!("[TEST] socket targeted wakeups");
    test_readable_change_does_not_wake_blocked_writer();
    crate::klogln!("[TEST] socket registration recheck");
    test_wait_future_rechecks_readiness();
    crate::klogln!("[TEST] socket timer restart");
    test_timed_read_restarts_timer_without_stale_wake();
}
