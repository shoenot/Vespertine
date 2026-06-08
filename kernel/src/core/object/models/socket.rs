use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::min;
use core::future::poll_fn;
use core::pin::Pin;
use core::sync::atomic::{
    AtomicBool, AtomicUsize, Ordering
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
use crate::core::asynchronous::async_sleep::{AsyncSleep, sleep_async};
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::core::sync::{
    Mutex,
    TicketLock,
};

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
pub struct SocketBus {
    pub buffer: Mutex<RingBuffer>,
    pub is_closed: AtomicBool,
    pub read_waker: TicketLock<Option<Waker>>,
    pub write_waker: TicketLock<Option<Waker>>,
    pub read_min: AtomicUsize,
    pub read_timeout_ds: AtomicUsize,
}

impl SocketBus {
    pub fn new() -> Self {
        Self {
            buffer: Mutex::new(RingBuffer::new()),
            is_closed: AtomicBool::new(false),
            read_waker: TicketLock::new(None),
            write_waker: TicketLock::new(None),
            read_min: AtomicUsize::new(1),
            read_timeout_ds: AtomicUsize::new(0),
        }
    }
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
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }
                self.read_with_policy(buffer_ptr, len).await
            }
            Invocation::File(FileOp::Write { buffer_ptr, len, .. }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                poll_fn(|cx| self.write_async(buffer_ptr as *mut u8, len, cx)).await
            }
            Invocation::Socket(SocketOp::SetNB { nb }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                self.is_nb.store(nb, Ordering::SeqCst);
                Ok(0)
            }
            Invocation::Socket(SocketOp::SetReadPolicy { min, timeout_ds }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                self.write_bus.read_min.store(min, Ordering::SeqCst);
                self.write_bus.read_timeout_ds.store(timeout_ds, Ordering::SeqCst);
                Ok(0)
            }
            Invocation::Wait(WaitOp::One(signal)) => {
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }
                poll_fn(|cx| self.wait_for_signals_async(signal, cx)).await
            }
            Invocation::Wait(WaitOp::Many { items_ptr: _, count: _ }) => {
                // invoke through ProcessControlBlock::invoke, not here manually
                Err(InvocationError::UnsupportedOperation)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl Drop for SocketEndpoint {
    fn drop(&mut self) {
        // mark the write bus as closed
        self.write_bus.is_closed.store(true, Ordering::SeqCst);

        // wake up any reading task waiting on the write bus
        if let Some(waker) = self.write_bus.read_waker.lock().take() {
            waker.wake();
        }

        // ditto for writing tasks
        if let Some(waker) = self.write_bus.write_waker.lock().take() {
            waker.wake();
        }
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

    async fn read_with_policy(
        &self,
        buffer_ptr: usize,
        len: usize,
    ) -> Result<usize, InvocationError> {
        if len == 0 {
            return Ok(0);
        }
    
        let min_bytes = self.read_bus.read_min.load(Ordering::SeqCst);
        let timeout_ds = self.read_bus.read_timeout_ds.load(Ordering::SeqCst);
    
        // reads currently use a 512-byte temporary buffer.
        let requested_min = min(min_bytes, len);
        let timeout_ms = timeout_ds.saturating_mul(100);
    
        let mut timer: Option<Pin<Box<AsyncSleep>>> =
            if requested_min == 0 && timeout_ms > 0 {
                // MIN=0, TIME>0: timer starts immediately.
                Some(Box::pin(sleep_async(timeout_ms)))
            } else {
                None
            };
    
        let mut last_available = 0usize;
    
        poll_fn(|cx| {
            let mut bus = self.read_bus.buffer.lock();
            let available = bus.len();
    
            if self.is_nb.load(Ordering::SeqCst) {
                if available == 0 {
                    return Poll::Ready(Err(InvocationError::WouldBlock));
                }
    
                return Poll::Ready(self.copy_from_read_bus(
                    &mut bus,
                    buffer_ptr,
                    len,
                ));
            }
    
            if self.read_bus.is_closed.load(Ordering::SeqCst) && available == 0 {
                return Poll::Ready(Ok(0));
            }
    
            let enough_data = if requested_min == 0 {
                available > 0
            } else {
                available >= requested_min
            };
    
            if enough_data {
                return Poll::Ready(self.copy_from_read_bus(
                    &mut bus,
                    buffer_ptr,
                    len,
                ));
            }
    
            if timeout_ms == 0 {
                // MIN=0, TIME=0: return immediately, including zero bytes.
                if requested_min == 0 {
                    return Poll::Ready(self.copy_from_read_bus(
                        &mut bus,
                        buffer_ptr,
                        len,
                    ));
                }
    
                // MIN>0, TIME=0: wait indefinitely for MIN bytes.
                *self.read_bus.read_waker.lock() = Some(cx.waker().clone());
                return Poll::Pending;
            }
    
            if requested_min > 0 && available > 0 && available != last_available {
                // MIN>0, TIME>0: inter-byte timer starts after the first
                // byte and restarts whenever additional bytes arrive.
                timer = Some(Box::pin(sleep_async(timeout_ms)));
                last_available = available;
            }
    
            if let Some(active_timer) = timer.as_mut() {
                if active_timer.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(self.copy_from_read_bus(
                        &mut bus,
                        buffer_ptr,
                        len,
                    ));
                }
            }
    
            *self.read_bus.read_waker.lock() = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    fn copy_from_read_bus(
        &self,
        bus: &mut RingBuffer,
        buffer_ptr: usize,
        len: usize,
    ) -> Result<usize, InvocationError> {
        let count = min(len, bus.len());
    
        let mut temp = Vec::new();
        temp.try_reserve_exact(count)
            .map_err(|_| InvocationError::OutOfMemory)?;
        temp.resize(count, 0);
    
        bus.pop_slice(&mut temp);
    
        if !safe_copy_to(buffer_ptr as *mut u8, temp.as_ptr(), count) {
            return Err(InvocationError::InvalidPointer);
        }
    
        if let Some(waker) = self.read_bus.write_waker.lock().take() {
            waker.wake();
        }
    
        Ok(count)
    }

    fn write_async(
        &self,
        buffer_ptr: *const u8,
        len: usize,
        cx: &mut Context<'_>,
    ) -> Poll<Result<usize, InvocationError>> {
        if len == 0 {
            return Poll::Ready(Ok(0));
        }
    
        if self.write_bus.is_closed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(InvocationError::UnsupportedOperation));
        }
    
        let mut bus = self.write_bus.buffer.lock();
        let count = min(len, bus.available_space());
    
        if count == 0 {
            if self.is_nb.load(Ordering::SeqCst) {
                return Poll::Ready(Err(InvocationError::WouldBlock));
            }
    
            *self.write_bus.write_waker.lock() = Some(cx.waker().clone());
            return Poll::Pending;
        }
    
        let mut temp = Vec::new();
        if temp.try_reserve_exact(count).is_err() {
            return Poll::Ready(Err(InvocationError::OutOfMemory));
        }
        temp.resize(count, 0);
    
        if !safe_copy_from(temp.as_mut_ptr(), buffer_ptr, count) {
            return Poll::Ready(Err(InvocationError::InvalidPointer));
        }
    
        let written = bus.push_slice(&temp);
    
        if let Some(waker) = self.write_bus.read_waker.lock().take() {
            waker.wake();
        }
    
        Poll::Ready(Ok(written))
    }

    fn wait_for_signals_async(&self, signal: Signal, cx: &mut Context<'_>) -> Poll<Result<usize, InvocationError>> {
        let mut should_block = false;
        let mut is_write = false;

        if signal.contains(Signal::READABLE) {
            let bus = self.read_bus.buffer.lock();
            if bus.is_empty() && !self.read_bus.is_closed.load(Ordering::SeqCst) {
                should_block = true;
                is_write = false;
            }
            drop(bus);
        }

        if signal.contains(Signal::WRITABLE) {
            let bus = self.write_bus.buffer.lock();
            if bus.is_full() && !self.write_bus.is_closed.load(Ordering::SeqCst) {
                should_block = true;
                is_write = true;
            }
            drop(bus);
        }

        if signal.contains(Signal::PEER_CLOSED) {
            let bus = self.read_bus.buffer.lock();
            if !self.read_bus.is_closed.load(Ordering::SeqCst) {
                should_block = true;
                is_write = false;
            }
            drop(bus);
        }

        if !should_block {
            return Poll::Ready(Ok(0));
        }

        if is_write {
            *self.write_bus.write_waker.lock() = Some(cx.waker().clone());
        } else {
            *self.read_bus.read_waker.lock() = Some(cx.waker().clone());
        }

        Poll::Pending
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
                if !calling_rights.contains(AccessRights::CREATE) {
                    return Err(InvocationError::AccessDenied);
                }
                let (ep1, ep2) = SocketEndpoint::new_pair();
                let current_proc = crate::core::thread::get_current_process().ok_or(InvocationError::OutOfMemory)?;

                let mut handles = current_proc.proc_handles.write();
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
    let mut handles = current_proc.proc_handles.write();
    let h1 = handles.insert(ep1, AccessRights::all());
    let h2 = handles.insert(ep2, AccessRights::all());
    (h1, h2)
}
