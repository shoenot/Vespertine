pub mod async_mutex;
pub mod async_sleep;
pub mod syscall_bridge;
pub mod waiter;
mod tests;
use alloc::boxed::Box;
use alloc::collections::vec_deque::VecDeque;
use alloc::sync::Arc;
use core::mem::forget;
use core::pin::Pin;
use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicBool,
    AtomicPtr,
    AtomicU8,
    AtomicUsize,
    Ordering,
};
use core::task::{
    Context,
    Poll,
    RawWaker,
    RawWakerVTable,
    Waker,
};

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
    interrupts_enabled,
};
use crate::core::sync::TicketLock;
use crate::core::thread::dispatch::{
    cancel_block_if_awoken,
    wake_thread,
};
use crate::core::thread::{
    ThreadControlBlock,
    ThreadState,
};

static TASK_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub static RUN_QUEUE: TicketLock<VecDeque<Arc<Task>>> = TicketLock::new(VecDeque::new());

pub static EXECUTOR_THREAD_PTR: AtomicPtr<ThreadControlBlock> = AtomicPtr::new(null_mut());
static EXECUTOR_AWOKEN: AtomicBool = AtomicBool::new(false);

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskState {
    Idle = 0,
    Queued = 1,
    Running = 2,
    RunningNotified = 3,
    Completed = 4,
}

pub struct Task {
    task_id: usize,
    state: AtomicU8,
    future: TicketLock<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl Task {
    pub fn new(future: impl Future<Output = ()> + 'static + Send) -> Self {
        let id = TASK_ID_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Self { task_id: id, state: AtomicU8::new(TaskState::Idle as u8), future: TicketLock::new(Box::pin(future)) }
    }

    pub fn poll(&self, context: &mut Context<'_>) -> Poll<()> {
        let mut future = self.future.lock();
        future.as_mut().poll(context)
    }

    pub fn id(&self) -> usize { self.task_id }
}

fn enqueue_task(task: Arc<Task>) {
    let mut queue = RUN_QUEUE.lock();
    queue.push_back(task);
    drop(queue);

    EXECUTOR_AWOKEN.store(true, Ordering::Release);
    let ptr = EXECUTOR_THREAD_PTR.load(Ordering::Acquire);
    if ptr.is_null() {
        return; // thread not registered yet
    }
    wake_thread(ptr);
}

fn poll_task(task: Arc<Task>) {
    task.state
        .compare_exchange(TaskState::Queued as u8, TaskState::Running as u8, Ordering::AcqRel, Ordering::Acquire)
        .expect("queued task was not queued");

    let waker = create_waker(task.clone());
    let mut context = Context::from_waker(&waker);

    match task.poll(&mut context) {
        Poll::Ready(()) => task.state.store(TaskState::Completed as u8, Ordering::Release),
        Poll::Pending => {
            if task.state.compare_exchange(TaskState::Running as u8, TaskState::Idle as u8, Ordering::AcqRel, Ordering::Acquire).is_err() {
                task.state
                    .compare_exchange(TaskState::RunningNotified as u8, TaskState::Queued as u8, Ordering::AcqRel, Ordering::Acquire)
                    .expect("pending task had invalid state");
                enqueue_task(task);
            }
        }
    }
}

pub fn wake_task(task: Arc<Task>) {
    loop {
        match task.state.load(Ordering::Acquire) {
            state if state == TaskState::Idle as u8 => {
                if task.state.compare_exchange(TaskState::Idle as u8, TaskState::Queued as u8, Ordering::AcqRel, Ordering::Acquire).is_ok()
                {
                    enqueue_task(task);
                    return;
                }
            }
            state if state == TaskState::Running as u8 => {
                if task
                    .state
                    .compare_exchange(TaskState::Running as u8, TaskState::RunningNotified as u8, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return;
                }
            }
            _ => return,
        }
    }
}

static TASK_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(task_waker_clone, task_waker_wake, task_waker_wake_by_ref, task_waker_drop);

unsafe fn task_waker_clone(data: *const ()) -> RawWaker {
    // reconstruct arc to clone it, +1 ref count
    let task = unsafe { Arc::from_raw(data as *const Task) };
    let cloned = task.clone();
    forget(task); // forget so the drop destructors dont run rn
    let raw = Arc::into_raw(cloned) as *const ();
    RawWaker::new(raw, &TASK_WAKER_VTABLE)
}

unsafe fn task_waker_wake(data: *const ()) {
    // reconstruct arc to take ownership of ptr
    let task = unsafe { Arc::from_raw(data as *const Task) };
    // push task back to rq to be polled again
    wake_task(task);
}

unsafe fn task_waker_wake_by_ref(data: *const ()) {
    // reconstruct arc, clone it, forget original
    let task = unsafe { Arc::from_raw(data as *const Task) };
    let cloned = task.clone();
    forget(task);
    wake_task(cloned);
}

unsafe fn task_waker_drop(data: *const ()) {
    // reconstruct and immediately drop arc to decrement refcount
    let _task = unsafe { Arc::from_raw(data as *const Task) };
}

pub fn create_waker(task: Arc<Task>) -> Waker {
    let raw = Arc::into_raw(task) as *const ();
    let raw_waker = RawWaker::new(raw, &TASK_WAKER_VTABLE);
    unsafe { Waker::from_raw(raw_waker) }
}

pub struct Executor;

impl Executor {
    pub fn new() -> Self { Self }

    /// spawn a generic future onto the global rq
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static + Send) {
        let task = Arc::new(Task::new(future));
        wake_task(task);
    }

    pub fn run(&self) -> ! {
        let tcb = get_core_data().scheduler.get_current_thread();
        EXECUTOR_THREAD_PTR.store(tcb, Ordering::Release);

        loop {
            let next_task = RUN_QUEUE.lock().pop_front();

            if let Some(task) = next_task {
                poll_task(task);
            } else {
                EXECUTOR_AWOKEN.store(false, Ordering::Release);

                // Hold the queue through the blocked transition so enqueue and wake cannot race it.
                let queue = RUN_QUEUE.lock();
                if queue.is_empty() {
                    let sched = &mut get_core_data().scheduler;
                    let current_thread = sched.get_current_thread();

                    let int_state = interrupts_enabled();
                    disable_interrupts();

                    unsafe { (*current_thread).transition(ThreadState::Running, ThreadState::Blocked) }
                        .expect("executor thread was not running");
                    drop(queue); // drop right before yield

                    if !cancel_block_if_awoken(unsafe { &*current_thread }, &EXECUTOR_AWOKEN) {
                        sched.schedule();
                    }
                    if int_state {
                        enable_interrupts();
                    }
                }
            }
        }
    }
}

pub extern "C" fn executor_thread(_arg: usize) -> ! { Executor::new().run() }

pub(crate) fn run_diagnostic_tests() { tests::run(); }
