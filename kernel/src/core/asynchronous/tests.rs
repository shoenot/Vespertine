use alloc::sync::Arc;
use alloc::task::Wake;
use core::future::Future;
use core::pin::Pin;
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

pub(super) fn run() {
    crate::klogln!("[TEST] executor repeated wakes");
    test_repeated_wakes_are_coalesced();
    crate::klogln!("[TEST] executor self wake");
    test_self_wake_is_polled_again();
    crate::klogln!("[TEST] executor completed future");
    test_completed_future_is_never_repolled();
    crate::klogln!("[TEST] async mutex contention");
    test_async_mutex_contenders_acquire();
    crate::klogln!("[TEST] async mutex cancelled waiter");
    test_dropped_async_mutex_waiter_is_skipped();
}
