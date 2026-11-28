use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use core::any::Any;
use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::Waker;

use async_trait::async_trait;
use hal::usercopy::{USER_SPACE_LIMIT, copy_to_user};
use vespertine_abi::{
    AccessRights, Invocation, IoCompletion, IoQueueOp, Signal, WaitOp,
};

use crate::executor::async_mutex::AsyncMutex;
use crate::executor::waiter::{AsyncWaiter, WaiterList, wake_all};
use crate::memory::pool::{PoolReservation, PoolState};
use crate::object::help::RightsWrapper;
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::object::obj::{KernelObject, ObjectWaitFuture};
use crate::sync::TicketLock;

const MAX_QUEUE_ENTRIES: usize = 64;
const MAX_TOTAL_ENTRIES: usize = 256;

static RESERVED_ENTRIES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct QueueBudget {
    entries: usize,
    _memory: PoolReservation,
}

impl Drop for QueueBudget {
    fn drop(&mut self) {
        RESERVED_ENTRIES.fetch_sub(self.entries, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct QueueState {
    pending: usize,
    completed: VecDeque<IoCompletion>,
    waiters: WaiterList,
}

#[derive(Debug)]
struct QueueInner {
    budget: Arc<QueueBudget>,
    state: TicketLock<QueueState>,
    consumer: AsyncMutex<()>,
}

#[derive(Debug)]
pub(crate) struct IoQueue {
    inner: Arc<QueueInner>,
}

pub(crate) struct CompletionSlot {
    queue: Option<Weak<QueueInner>>,
    _budget: Arc<QueueBudget>,
}

impl IoQueue {
    pub fn new(entries: usize, pool: &Arc<PoolState>) -> Result<Self, InvocationError> {
        if entries == 0 || entries > MAX_QUEUE_ENTRIES { return Err(InvocationError::InvalidArgument) };

        let memory = pool.reserve(size_of::<QueueInner>() + entries * size_of::<IoCompletion>())?;

        let mut completed = VecDeque::new();
        completed.try_reserve_exact(entries).map_err(|_| InvocationError::OutOfMemory)?;

        RESERVED_ENTRIES.fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            let total = used + entries;
            (total <= MAX_TOTAL_ENTRIES).then_some(total)
        }).map_err(|_| InvocationError::PoolExhausted)?;

        let budget = Arc::new(QueueBudget { entries, _memory: memory });
        Ok(Self {
            inner: Arc::new(QueueInner {
                budget,
                state: TicketLock::new(QueueState {
                    pending: 0,
                    completed,
                    waiters: WaiterList::new(),
                }),
                consumer: AsyncMutex::new(()),
            }),
        })
    }

    pub fn reserve_completion(&self) -> Result<CompletionSlot, InvocationError> {
        let mut state = self.inner.state.lock();
        if state.pending + state.completed.len() == self.inner.budget.entries {
            return Err(InvocationError::WouldBlock);
        }

        state.pending += 1;
        Ok(CompletionSlot {
            queue: Some(Arc::downgrade(&self.inner)),
            _budget: Arc::clone(&self.inner.budget),
        })
    }

    async fn pop(&self, ptr: usize) -> Result<usize, InvocationError> {
        let len = size_of::<IoCompletion>();
        if ptr == 0 || ptr.checked_add(len).is_none_or(|end| end > USER_SPACE_LIMIT) {
            return Err(InvocationError::InvalidPointer);
        }

        // serialize consumers across the copy, producers can still append
        let _consumer = self.inner.consumer.lock().await;
        let completion = self.inner.state.lock().completed.front().copied()
            .ok_or(InvocationError::WouldBlock)?;

        if !copy_to_user(ptr as *mut u8, (&completion as *const IoCompletion).cast(), len) {
            return Err(InvocationError::InvalidPointer);
        }

        self.inner.state.lock().completed.pop_front();
        Ok(0)
    }
}

impl CompletionSlot {
    pub fn complete(mut self, completion: IoCompletion) {
        let Some(queue) = self.queue.take().and_then(|queue| queue.upgrade()) else {
            return;
        };

        let wakers = {
            let mut state = queue.state.lock();
            state.pending -= 1;
            state.completed.push_back(completion);
            state.waiters.take_wakers()
        };

        wake_all(wakers);
    }
}

impl Drop for CompletionSlot {
    fn drop(&mut self) {
        // roll back if submission failed before the task was enqueued
        if let Some(queue) = self.queue.take().and_then(|queue| queue.upgrade()) {
            queue.state.lock().pending -= 1;
        }
    }
}

#[async_trait]
impl KernelObject for IoQueue {
    fn type_name(&self) -> &'static str { "IoQueue" }

    fn as_any(&self) -> &dyn Any { self }

    fn current_signals(&self) -> Signal {
        if self.inner.state.lock().completed.is_empty() {
            Signal(0)
        } else {
            Signal::READABLE
        }
    }

    fn register_waiter(&self, requested: Signal, waiter: &Arc<AsyncWaiter>, waker: &Waker) -> Result<(), InvocationError> {
        if requested != Signal::READABLE { return Err(InvocationError::UnsupportedOperation) };

        self.inner.state.lock().waiters.register(waiter, waker);
        Ok(())
    }

    async fn invoke(&self, invocation: Invocation, rights: AccessRights, ctx: InvokeContext<'_>) -> Result<usize, InvocationError> {
        rights.err_if_no(AccessRights::READ)?;

        match invocation {
            Invocation::IoQueue(IoQueueOp::Pop { completion_ptr }) => {
                self.pop(completion_ptr).await
            }
            Invocation::Wait(WaitOp::One(signal)) if signal == Signal::READABLE => {
                ctx.wait(ObjectWaitFuture::new(self, signal)).await
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
