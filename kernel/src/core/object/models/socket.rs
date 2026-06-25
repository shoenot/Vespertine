use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::min;
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
    Waker,
};

use async_trait::async_trait;
use vespertine_abi::op::{
    FileOp,
    SocketOp,
};
use vespertine_abi::{
    AccessRights,
    HandleID,
    Invocation,
    Signal,
    WaitOp,
};

use crate::arch::x86_64::task::syscall::{
    safe_copy_from,
    safe_copy_to,
};
use crate::core::asynchronous::async_sleep::{
    AsyncSleep,
    sleep_async,
};
use crate::core::asynchronous::waiter::{
    AsyncWaiter,
    WaiterList,
    wake_all,
};
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::{
    KernelObject,
    ObjectWaitFuture,
    matching_signals,
};
use crate::core::sync::Mutex;

#[path = "socket_tests.rs"]
mod tests;

const BUFFER_SIZE: usize = 4096;

#[derive(Debug)]
pub struct RingBuffer {
    data: [u8; BUFFER_SIZE],
    head: usize,
    tail: usize,
}

impl RingBuffer {
    pub const fn new() -> Self { Self { data: [0; BUFFER_SIZE], head: 0, tail: 0 } }

    pub fn is_empty(&self) -> bool { self.head == self.tail }

    pub fn is_full(&self) -> bool { ((self.head + 1) % BUFFER_SIZE) == self.tail }

    pub fn len(&self) -> usize { if self.head >= self.tail { self.head - self.tail } else { BUFFER_SIZE - (self.tail - self.head) } }

    pub fn available_space(&self) -> usize { if self.is_full() { 0 } else { BUFFER_SIZE - self.len() - 1 } }

    pub fn push_slice(&mut self, src: &[u8]) -> usize {
        let n = min(src.len(), self.available_space());
        for i in 0..n {
            self.data[self.head] = src[i];
            self.head = (self.head + 1) % BUFFER_SIZE;
        }
        n
    }

    pub fn pop_slice(&mut self, dst: &mut [u8]) -> usize {
        let n = min(dst.len(), self.len());
        for i in 0..n {
            dst[i] = self.data[self.tail];
            self.tail = (self.tail + 1) % BUFFER_SIZE;
        }
        n
    }
}

#[derive(Debug)]
pub struct SocketBusInner {
    pub buffer: RingBuffer,
    pub read_waiters: WaiterList,
    pub write_waiters: WaiterList,
    pub readable_signal_waiters: WaiterList,
    pub writable_signal_waiters: WaiterList,
}

#[derive(Debug)]
pub struct SocketBus {
    pub inner: Mutex<SocketBusInner>,
    pub is_closed: AtomicBool,
    pub read_min: AtomicUsize,
    pub read_timeout_ds: AtomicUsize,
}

impl SocketBus {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(SocketBusInner {
                buffer: RingBuffer::new(),
                read_waiters: WaiterList::new(),
                write_waiters: WaiterList::new(),
                readable_signal_waiters: WaiterList::new(),
                writable_signal_waiters: WaiterList::new(),
            }),
            is_closed: AtomicBool::new(false),
            read_min: AtomicUsize::new(1),
            read_timeout_ds: AtomicUsize::new(0),
        }
    }

    pub fn notify_readers(&self) {
        let wakers = self.inner.lock().read_waiters.take_wakers();
        wake_all(wakers);
    }

    pub fn notify_writers(&self) {
        let wakers = self.inner.lock().write_waiters.take_wakers();
        wake_all(wakers);
    }

    pub fn notify_readable(&self) {
        let wakers = {
            let mut inner = self.inner.lock();
            let mut wakers = inner.read_waiters.take_wakers();
            wakers.extend(inner.readable_signal_waiters.take_wakers());
            wakers
        };
        wake_all(wakers);
    }

    pub fn notify_writable(&self) {
        let wakers = {
            let mut inner = self.inner.lock();
            let mut wakers = inner.write_waiters.take_wakers();
            wakers.extend(inner.writable_signal_waiters.take_wakers());
            wakers
        };
        wake_all(wakers);
    }

    pub fn notify_all(&self) {
        let wakers = {
            let mut inner = self.inner.lock();
            let mut wakers = inner.read_waiters.take_wakers();
            wakers.extend(inner.write_waiters.take_wakers());
            wakers.extend(inner.readable_signal_waiters.take_wakers());
            wakers.extend(inner.writable_signal_waiters.take_wakers());
            wakers
        };
        wake_all(wakers);
    }
}

struct SocketReadFuture<'a> {
    endpoint: &'a SocketEndpoint,
    buffer_ptr: usize,
    len: usize,
    requested_min: usize,
    timeout_ms: usize,
    timer: Option<Pin<Box<AsyncSleep>>>,
    last_available: usize,
    waiter: Arc<AsyncWaiter>,
}

impl<'a> SocketReadFuture<'a> {
    fn new(endpoint: &'a SocketEndpoint, buffer_ptr: usize, len: usize, requested_min: usize, timeout_ds: usize) -> Self {
        let timeout_ms = timeout_ds.saturating_mul(100);
        let timer = if requested_min == 0 && timeout_ms > 0 { Some(Box::pin(sleep_async(timeout_ms))) } else { None };
        Self { endpoint, buffer_ptr, len, requested_min, timeout_ms, timer, last_available: 0, waiter: AsyncWaiter::new() }
    }
}

impl Drop for SocketReadFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for SocketReadFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        let mut inner = this.endpoint.read_bus.inner.lock();
        let available = inner.buffer.len();

        if this.endpoint.is_nb.load(Ordering::Acquire) && available == 0 {
            return Poll::Ready(Err(InvocationError::WouldBlock));
        }

        if this.endpoint.read_bus.is_closed.load(Ordering::Acquire) && available == 0 {
            return Poll::Ready(Ok(0));
        }

        let enough_data = if this.requested_min == 0 { available > 0 } else { available >= this.requested_min };
        let immediate_empty_read = this.requested_min == 0 && this.timeout_ms == 0;

        if enough_data || immediate_empty_read {
            let count = core::cmp::min(this.len, available);
            let mut temp = Vec::new();

            if temp.try_reserve_exact(count).is_err() {
                return Poll::Ready(Err(InvocationError::OutOfMemory));
            }
            temp.resize(count, 0);

            inner.buffer.pop_slice(&mut temp);
            drop(inner);

            if !safe_copy_to(this.buffer_ptr as *mut u8, temp.as_ptr(), count) {
                return Poll::Ready(Err(InvocationError::InvalidPointer));
            }

            this.endpoint.read_bus.notify_writable();
            return Poll::Ready(Ok(count));
        }

        inner.read_waiters.register(&this.waiter, cx.waker());

        if this.requested_min > 0 && this.timeout_ms > 0 && available > 0 && available != this.last_available {
            this.timer = Some(Box::pin(sleep_async(this.timeout_ms)));
            this.last_available = available;
        }
        drop(inner);

        if let Some(timer) = this.timer.as_mut() {
            if timer.as_mut().poll(cx).is_ready() {
                this.waiter.deactivate();
                let mut inner = this.endpoint.read_bus.inner.lock();
                let count = min(this.len, inner.buffer.len());
                let mut temp = Vec::new();
                if temp.try_reserve_exact(count).is_err() {
                    return Poll::Ready(Err(InvocationError::OutOfMemory));
                }
                temp.resize(count, 0);
                inner.buffer.pop_slice(&mut temp);
                drop(inner);

                if !safe_copy_to(this.buffer_ptr as *mut u8, temp.as_ptr(), count) {
                    return Poll::Ready(Err(InvocationError::InvalidPointer));
                }
                this.endpoint.read_bus.notify_writable();
                return Poll::Ready(Ok(count));
            }
        }

        Poll::Pending
    }
}

impl<'a> SocketWriteFuture<'a> {
    fn new(endpoint: &'a SocketEndpoint, buffer_ptr: usize, len: usize) -> Self {
        Self { endpoint, buffer_ptr, len, waiter: AsyncWaiter::new() }
    }
}

impl Drop for SocketWriteFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for SocketWriteFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.endpoint.write_bus.is_closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(InvocationError::UnsupportedOperation));
        }

        let mut inner = this.endpoint.write_bus.inner.lock();
        let count = min(this.len, inner.buffer.available_space());

        if count == 0 {
            if this.endpoint.is_nb.load(Ordering::Acquire) {
                return Poll::Ready(Err(InvocationError::WouldBlock));
            }

            inner.write_waiters.register(&this.waiter, cx.waker());
            return Poll::Pending;
        }

        let mut temp = Vec::new();
        if temp.try_reserve_exact(count).is_err() {
            return Poll::Ready(Err(InvocationError::OutOfMemory));
        }
        temp.resize(count, 0);

        if !safe_copy_from(temp.as_mut_ptr(), this.buffer_ptr as *const u8, count) {
            return Poll::Ready(Err(InvocationError::InvalidPointer));
        }

        let written = inner.buffer.push_slice(&temp);
        drop(inner);

        this.endpoint.write_bus.notify_readable();

        Poll::Ready(Ok(written))
    }
}

struct SocketWriteFuture<'a> {
    endpoint: &'a SocketEndpoint,
    buffer_ptr: usize,
    len: usize,
    waiter: Arc<AsyncWaiter>,
}

struct SocketWaitFuture<'a> {
    endpoint: &'a SocketEndpoint,
    requested: Signal,
    waiter: Arc<AsyncWaiter>,
}

impl SocketWaitFuture<'_> {
    fn poll_with_registration_hook(
        &mut self, cx: &mut Context<'_>, after_registration: impl FnOnce(),
    ) -> Poll<Result<usize, InvocationError>> {
        let matched = matching_signals(self.endpoint.current_signals(), self.requested);
        if matched != Signal(0) {
            return Poll::Ready(Ok(0));
        }

        if self.requested.contains(Signal::READABLE) || self.requested.contains(Signal::PEER_CLOSED) {
            self.endpoint.read_bus.inner.lock().readable_signal_waiters.register(&self.waiter, cx.waker());
        }
        if self.requested.contains(Signal::WRITABLE) {
            self.endpoint.write_bus.inner.lock().writable_signal_waiters.register(&self.waiter, cx.waker());
        }

        after_registration();

        let matched = matching_signals(self.endpoint.current_signals(), self.requested);
        if matched != Signal(0) {
            self.waiter.deactivate();
            Poll::Ready(Ok(0))
        } else {
            Poll::Pending
        }
    }
}

impl Drop for SocketWaitFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for SocketWaitFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> { self.get_mut().poll_with_registration_hook(cx, || {}) }
}

#[derive(Debug)]
pub struct SocketEndpoint {
    pub read_bus: Arc<SocketBus>,
    pub write_bus: Arc<SocketBus>,
    pub is_nb: AtomicBool,
}

#[async_trait]
impl KernelObject for SocketEndpoint {
    fn type_name(&self) -> &'static str { "Socket" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { buffer_ptr, len, .. }) => {
                calling_rights.err_if_no(AccessRights::READ)?;
                if len == 0 {
                    return Ok(0);
                }
                let requested_min = min(self.read_bus.read_min.load(Ordering::Acquire), len);
                let timeout_ds = self.read_bus.read_timeout_ds.load(Ordering::Acquire);
                SocketReadFuture::new(self, buffer_ptr, len, requested_min, timeout_ds).await
            }
            Invocation::File(FileOp::Write { buffer_ptr, len, .. }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                if len == 0 {
                    return Ok(0);
                }
                SocketWriteFuture::new(self, buffer_ptr, len).await
            }
            Invocation::Socket(SocketOp::SetNB { nb }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.is_nb.store(nb, Ordering::Release);
                self.read_bus.notify_readers();
                self.write_bus.notify_writers();
                Ok(0)
            }
            Invocation::Socket(SocketOp::SetReadPolicy { min, timeout_ds }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.write_bus.read_min.store(min, Ordering::Release);
                self.write_bus.read_timeout_ds.store(timeout_ds, Ordering::Release);
                self.write_bus.notify_readers();
                Ok(0)
            }
            Invocation::Wait(WaitOp::One(signal)) => ObjectWaitFuture::new(self, signal).await,
            Invocation::Wait(WaitOp::Many { items_ptr: _, count: _ }) => {
                // invoke through ProcessControlBlock::invoke, not here manually
                Err(InvocationError::UnsupportedOperation)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn current_signals(&self) -> Signal { SocketEndpoint::current_signals(self) }

    fn register_waiter(&self, requested: Signal, waiter: &Arc<AsyncWaiter>, waker: &Waker) -> Result<(), InvocationError> {
        let supported = Signal::READABLE | Signal::WRITABLE | Signal::PEER_CLOSED;
        if requested != (requested & supported) {
            return Err(InvocationError::UnsupportedOperation);
        }

        if requested.contains(Signal::READABLE) || requested.contains(Signal::PEER_CLOSED) {
            self.read_bus.inner.lock().readable_signal_waiters.register(waiter, waker);
        }

        if requested.contains(Signal::WRITABLE) {
            self.write_bus.inner.lock().writable_signal_waiters.register(waiter, waker);
        }

        Ok(())
    }
}

impl Drop for SocketEndpoint {
    fn drop(&mut self) {
        self.write_bus.is_closed.store(true, Ordering::Release);
        self.write_bus.notify_readable();
    }
}

impl SocketEndpoint {
    pub fn new_pair() -> (Arc<SocketEndpoint>, Arc<SocketEndpoint>) {
        let bus1 = Arc::new(SocketBus::new());
        let bus2 = Arc::new(SocketBus::new());

        let ep1 = Arc::new(SocketEndpoint { read_bus: bus1.clone(), write_bus: bus2.clone(), is_nb: AtomicBool::new(false) });

        let ep2 = Arc::new(SocketEndpoint { read_bus: bus2, write_bus: bus1, is_nb: AtomicBool::new(false) });

        (ep1, ep2)
    }

    pub(crate) fn current_signals(&self) -> Signal {
        let mut signals = Signal(0);
        {
            let inner = self.read_bus.inner.lock();
            if !inner.buffer.is_empty() || self.read_bus.is_closed.load(Ordering::Acquire) {
                signals = signals | Signal::READABLE;
            }
        }
        {
            let inner = self.write_bus.inner.lock();
            if !inner.buffer.is_full() && !self.write_bus.is_closed.load(Ordering::Acquire) {
                signals = signals | Signal::WRITABLE;
            }
        }
        if self.read_bus.is_closed.load(Ordering::Acquire) {
            signals = signals | Signal::PEER_CLOSED;
        }
        signals
    }

    pub(crate) async fn write_all_internal(&self, data: &[u8]) -> Result<(), InvocationError> {
        let mut written = 0;
        while written < data.len() {
            let count = InternalSocketWrite { endpoint: self, data: &data[written..], waiter: AsyncWaiter::new() }.await?;

            if count == 0 {
                return Err(InvocationError::UnsupportedOperation);
            }

            written += count;
        }
        Ok(())
    }
}

pub struct InternalSocketWrite<'a> {
    endpoint: &'a SocketEndpoint,
    data: &'a [u8],
    waiter: Arc<AsyncWaiter>,
}

impl Drop for InternalSocketWrite<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for InternalSocketWrite<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.endpoint.write_bus.is_closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(InvocationError::UnsupportedOperation));
        }

        let mut inner = this.endpoint.write_bus.inner.lock();
        if inner.buffer.is_full() {
            inner.write_waiters.register(&this.waiter, cx.waker());
            return Poll::Pending;
        }

        let count = inner.buffer.push_slice(this.data);
        drop(inner);
        this.endpoint.write_bus.notify_readable();
        Poll::Ready(Ok(count))
    }
}

#[derive(Debug)]
pub struct SocketFactory {}

#[async_trait]
impl KernelObject for SocketFactory {
    fn type_name(&self) -> &'static str { "SocketFactory" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Socket(SocketOp::Create { .. }) => {
                calling_rights.err_if_no(AccessRights::CREATE)?;
                let (ep1, ep2) = SocketEndpoint::new_pair();
                let current_proc = crate::core::thread::get_current_process().ok_or(InvocationError::OutOfMemory)?;

                let mut handles = current_proc.handles.write();
                let h1 = handles.insert(ep1, AccessRights::all());
                let h2 = handles.insert(ep2, AccessRights::all());

                // Pack both handles into return value: low 32 = h1, high 32 = h2
                Ok((h1.0 & 0xFFFFFFFF) | ((h2.0 & 0xFFFFFFFF) << 32))
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

pub fn init_ipc_pipeline() -> (HandleID, HandleID) {
    let (ep1, ep2) = SocketEndpoint::new_pair();
    let current_proc = crate::core::thread::get_current_process().expect("No current process during IPC init");
    let mut handles = current_proc.handles.write();
    let h1 = handles.insert(ep1, AccessRights::all());
    let h2 = handles.insert(ep2, AccessRights::all());
    (h1, h2)
}

pub(crate) fn run_diagnostic_tests() { tests::run(); }
