use alloc::sync::Arc;
use alloc::task::Wake;
use core::future::{Future, pending, poll_fn};
use core::pin::{pin, Pin};
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};
use core::task::{
    Context,
    Poll,
};

use super::async_mutex::AsyncMutex;
use super::async_sleep::run_diagnostic_tests as run_async_sleep_tests;
use super::waiter::{TerminationToken, Terminating, interruptible_wait, wake_all};
use super::{
    RUN_QUEUE,
    Task,
    TaskState,
    poll_task,
    wake_task,
};

struct CountingWaker {
    wakes: AtomicUsize,
}

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) { self.wake_by_ref(); }

    fn wake_by_ref(self: &Arc<Self>) { self.wakes.fetch_add(1, Ordering::Relaxed); }
}

fn counting_context() -> (Arc<CountingWaker>, core::task::Waker) {
    let counter = Arc::new(CountingWaker { wakes: AtomicUsize::new(0) });
    let waker = counter.clone().into();
    (counter, waker)
}

fn pop_task() -> Arc<Task> { RUN_QUEUE.lock().pop_front().expect("expected queued task") }

fn assert_queue_empty() {
    assert!(RUN_QUEUE.lock().is_empty(), "executor test leaked a queued task");
}

struct RepeatedWakeFuture {
    polling: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
}

impl Future for RepeatedWakeFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        assert!(!self.polling.swap(true, Ordering::AcqRel), "future was polled concurrently");
        let poll = self.polls.fetch_add(1, Ordering::Relaxed);
        if poll == 0 {
            for _ in 0..64 {
                cx.waker().wake_by_ref();
            }
        }
        self.polling.store(false, Ordering::Release);
        if poll == 0 { Poll::Pending } else { Poll::Ready(()) }
    }
}

struct SelfWakingFuture {
    polls: Arc<AtomicUsize>,
}

impl Future for SelfWakingFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.polls.fetch_add(1, Ordering::Relaxed) == 0 {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

struct CompletedFuture {
    polls: Arc<AtomicUsize>,
}

impl Future for CompletedFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        Poll::Ready(())
    }
}

fn test_repeated_wakes_are_coalesced() {
    assert_queue_empty();
    let polling = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let task = Arc::new(Task::new(RepeatedWakeFuture { polling, polls: polls.clone() }));

    for _ in 0..64 {
        wake_task(task.clone());
    }
    assert_eq!(RUN_QUEUE.lock().len(), 1, "duplicate wakes created duplicate queue entries");

    poll_task(pop_task());
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_eq!(task.state.load(Ordering::Acquire), TaskState::Queued as u8);
    assert_eq!(RUN_QUEUE.lock().len(), 1, "wakes during poll created duplicate queue entries");

    poll_task(pop_task());
    assert_eq!(polls.load(Ordering::Acquire), 2);
    assert_eq!(task.state.load(Ordering::Acquire), TaskState::Completed as u8);
    assert_queue_empty();
}

fn test_self_wake_is_polled_again() {
    assert_queue_empty();
    let polls = Arc::new(AtomicUsize::new(0));
    let task = Arc::new(Task::new(SelfWakingFuture { polls: polls.clone() }));

    wake_task(task.clone());
    poll_task(pop_task());
    assert_eq!(task.state.load(Ordering::Acquire), TaskState::Queued as u8);
    poll_task(pop_task());

    assert_eq!(polls.load(Ordering::Acquire), 2);
    assert_eq!(task.state.load(Ordering::Acquire), TaskState::Completed as u8);
    assert_queue_empty();
}

fn test_completed_future_is_never_repolled() {
    assert_queue_empty();
    let polls = Arc::new(AtomicUsize::new(0));
    let task = Arc::new(Task::new(CompletedFuture { polls: polls.clone() }));

    wake_task(task.clone());
    poll_task(pop_task());
    for _ in 0..64 {
        wake_task(task.clone());
    }

    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_eq!(task.state.load(Ordering::Acquire), TaskState::Completed as u8);
    assert_queue_empty();
}

fn poll_lock<'a, T>(
    future: &mut super::async_mutex::AsyncMutexLockFuture<'a, T>, context: &mut Context<'_>,
) -> Poll<super::async_mutex::AsyncMutexGuard<'a, T>> {
    Pin::new(future).poll(context)
}

fn test_async_mutex_contenders_acquire() {
    let mutex = AsyncMutex::new(0usize);
    let (_counter, waker) = counting_context();
    let mut context = Context::from_waker(&waker);

    let mut first = mutex.lock();
    let mut first_guard = match poll_lock(&mut first, &mut context) {
        Poll::Ready(guard) => guard,
        Poll::Pending => panic!("first mutex contender did not acquire"),
    };
    *first_guard += 1;

    let mut second = mutex.lock();
    assert!(poll_lock(&mut second, &mut context).is_pending());
    drop(first_guard);

    let mut second_guard = match poll_lock(&mut second, &mut context) {
        Poll::Ready(guard) => guard,
        Poll::Pending => panic!("second mutex contender did not acquire"),
    };
    *second_guard += 1;
    drop(second_guard);

    assert_eq!(*mutex.try_lock().expect("mutex remained locked"), 2);
}

fn test_dropped_async_mutex_waiter_is_skipped() {
    let mutex = AsyncMutex::new(());
    let (counter, waker) = counting_context();
    let mut context = Context::from_waker(&waker);

    let mut owner = mutex.lock();
    let owner_guard = match poll_lock(&mut owner, &mut context) {
        Poll::Ready(guard) => guard,
        Poll::Pending => panic!("mutex owner did not acquire"),
    };

    let mut cancelled = mutex.lock();
    assert!(poll_lock(&mut cancelled, &mut context).is_pending());
    let mut later = mutex.lock();
    assert!(poll_lock(&mut later, &mut context).is_pending());
    drop(cancelled);
    drop(owner_guard);

    assert_eq!(counter.wakes.load(Ordering::Acquire), 1, "later waiter was not woken");
    let later_guard = match poll_lock(&mut later, &mut context) {
        Poll::Ready(guard) => guard,
        Poll::Pending => panic!("later waiter was blocked by cancelled waiter"),
    };
    drop(later_guard);
}

fn test_prior_termination_skips_inner_poll() {
    let token = TerminationToken::new();
    assert!(!token.is_requested());
    assert!(token.request_and_take_wakers().is_empty());
    assert!(token.is_requested());

    let (_, waker) = counting_context();
    let mut cx = Context::from_waker(&waker);
    let inner = poll_fn(|_| -> Poll<()> {
        panic!("polled the operation after termination was already requested");
    });
    let mut wait = pin!(interruptible_wait(&token, inner));

    assert_eq!(wait.as_mut().poll(&mut cx), Poll::Ready(Err(Terminating)));
}

fn test_termination_between_inner_poll_and_registration() {
    let token = TerminationToken::new();
    let (counter, waker) = counting_context();
    let mut cx = Context::from_waker(&waker);

    // exact ordering: the wrapper has checked the token, but
    // hasn't yet registered its cancellation waiter
    let inner = poll_fn(|_| {
        assert!(token.request_and_take_wakers().is_empty());
        Poll::<()>::Pending
    });
    let mut wait = pin!(interruptible_wait(&token, inner));

    assert_eq!(wait.as_mut().poll(&mut cx), Poll::Ready(Err(Terminating)));
    assert_eq!(counter.wakes.load(Ordering::Relaxed), 0);
}

fn test_ready_result_survives_termination_during_poll() {
    let token = TerminationToken::new();
    let (_, waker) = counting_context();
    let mut cx = Context::from_waker(&waker);

    // model a socket write that committed 64 bytes during this poll
    let inner = poll_fn(|_| {
        assert!(token.request_and_take_wakers().is_empty());
        Poll::Ready(64usize)
    });
    let mut wait = pin!(interruptible_wait(&token, inner));

    assert_eq!(wait.as_mut().poll(&mut cx), Poll::Ready(Ok(64)));
    assert!(token.is_requested());
}

fn test_termination_wakes_each_live_wait_once() {
    let token = TerminationToken::new();
    let (first_counter, first_waker) = counting_context();
    let (second_counter, second_waker) = counting_context();
    let mut first_cx = Context::from_waker(&first_waker);
    let mut second_cx = Context::from_waker(&second_waker);

    let mut first = pin!(interruptible_wait(&token, pending::<()>()));
    let mut second = pin!(interruptible_wait(&token, pending::<()>()));

    assert!(first.as_mut().poll(&mut first_cx).is_pending());
    assert!(second.as_mut().poll(&mut second_cx).is_pending());

    let wakes = token.request_and_take_wakers();
    assert!(token.is_requested());
    assert_eq!(wakes.len(), 2);
    assert!(token.request_and_take_wakers().is_empty());
    assert_eq!(first_counter.wakes.load(Ordering::Relaxed), 0);
    assert_eq!(second_counter.wakes.load(Ordering::Relaxed), 0);

    wake_all(wakes);
    assert_eq!(first_counter.wakes.load(Ordering::Relaxed), 1);
    assert_eq!(second_counter.wakes.load(Ordering::Relaxed), 1);
    assert_eq!(first.as_mut().poll(&mut first_cx), Poll::Ready(Err(Terminating)));
    assert_eq!(second.as_mut().poll(&mut second_cx), Poll::Ready(Err(Terminating)));
}

fn test_repoll_replaces_termination_waker() {
    let token = TerminationToken::new();
    let (old_counter, old_waker) = counting_context();
    let (new_counter, new_waker) = counting_context();
    let mut wait = pin!(interruptible_wait(&token, pending::<()>()));

    assert!(wait.as_mut().poll(&mut Context::from_waker(&old_waker)).is_pending());
    assert!(wait.as_mut().poll(&mut Context::from_waker(&new_waker)).is_pending());

    let wakes = token.request_and_take_wakers();
    assert_eq!(wakes.len(), 1);
    wake_all(wakes);

    assert_eq!(old_counter.wakes.load(Ordering::Relaxed), 0);
    assert_eq!(new_counter.wakes.load(Ordering::Relaxed), 1);
}

fn test_dropped_wait_receives_no_termination_wake() {
    let token = TerminationToken::new();
    let (counter, waker) = counting_context();
    let mut cx = Context::from_waker(&waker);

    {
        let mut wait = pin!(interruptible_wait(&token, pending::<()>()));
        assert!(wait.as_mut().poll(&mut cx).is_pending());
    } // drop the underlying future, including its Arc<AsyncWaiter>

    let wakes = token.request_and_take_wakers();
    assert!(wakes.is_empty());
    assert_eq!(counter.wakes.load(Ordering::Relaxed), 0);
}

pub(super) fn run() {
    crate::klogln!(Info, "Test/Executor", "Repeated wakes");
    test_repeated_wakes_are_coalesced();
    crate::klogln!(Info, "Test/Executor", "Self wake");
    test_self_wake_is_polled_again();
    crate::klogln!(Info, "Test/Executor", "Completed future");
    test_completed_future_is_never_repolled();
    crate::klogln!(Info, "Test/Executor", "Async mutex contention");
    test_async_mutex_contenders_acquire();
    crate::klogln!(Info, "Test/Executor", "Async mutex cancelled waiter");
    test_dropped_async_mutex_waiter_is_skipped();
    crate::klogln!(Info, "Test/Executor", "Termination token and interruptible waits");
    test_prior_termination_skips_inner_poll();
    test_termination_between_inner_poll_and_registration();
    test_ready_result_survives_termination_during_poll();
    test_termination_wakes_each_live_wait_once();
    test_repoll_replaces_termination_waker();
    test_dropped_wait_receives_no_termination_wake();
    crate::klogln!(Info, "Test/Async", "Async sleep tests");
    run_async_sleep_tests();
}
