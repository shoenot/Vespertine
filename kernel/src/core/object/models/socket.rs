use core::cmp::min;
use core::sync::atomic::{AtomicBool, Ordering};
use alloc::sync::Arc;
use crate::arch::{disable_interrupts, enable_interrupts, get_core_data, interrupts_enabled};
use crate::core::sync::{Mutex, Semaphore, TicketLock};
use crate::core::object::obj::KernelObject;
use crate::core::object::invoke::InvocationError;
use crate::core::thread::ThreadState;
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::wait::WaitQueue;
use vespertine_abi::{HandleID, Invocation, Signal};
use vespertine_abi::op::{FileOp, SocketOp};
use vespertine_abi::AccessRights;
use crate::arch::x86_64::task::syscall::{safe_copy_from, safe_copy_to};

const BUFFER_SIZE: usize = 4096;

#[derive(Debug)]
pub struct RingBuffer {
    data: [u8; BUFFER_SIZE],
    head: usize,
    tail: usize,
}

impl RingBuffer {
    pub const fn new() -> Self {
        Self { data: [0; BUFFER_SIZE], head: 0, tail: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    pub fn is_full(&self) -> bool {
        ((self.head + 1) % BUFFER_SIZE) == self.tail
    }

    pub fn len(&self) -> usize {
        if self.head >= self.tail {
            self.head - self.tail
        } else {
            BUFFER_SIZE - (self.tail - self.head)
        }
    }

    pub fn available_space(&self) -> usize {
        if self.is_full() {
            0
        } else {
            BUFFER_SIZE - self.len() - 1
        }
    }

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
    pub semaphore: Semaphore,
    pub is_closed: AtomicBool,
    pub read_waiters: TicketLock<WaitQueue>,
    pub write_waiters: TicketLock<WaitQueue>,
}

impl SocketBus {
    pub fn new() -> Self {
        Self {
            buffer: Mutex::new(RingBuffer::new()),
            semaphore: Semaphore::new(0),
            is_closed: AtomicBool::new(false),
            read_waiters: TicketLock::new(WaitQueue::new()),
            write_waiters: TicketLock::new(WaitQueue::new()),
        }
    }
}

#[derive(Debug)]
pub struct SocketEndpoint {
    pub read_bus: Arc<SocketBus>,
    pub write_bus: Arc<SocketBus>,
    pub is_nb: AtomicBool,
}

impl KernelObject for SocketEndpoint {
    fn type_name(&self) -> &'static str {
        "Socket"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { buffer_ptr, len, .. }) => {
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }
                self.read(buffer_ptr, len)
            },
            Invocation::File(FileOp::Write { buffer_ptr, len, .. }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                self.write(buffer_ptr, len)
            },
            Invocation::Socket(SocketOp::SetNB { nb }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                self.is_nb.store(nb, Ordering::SeqCst);
                Ok(0)
            },
            Invocation::Wait(signal) => {
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }
                self.wait_for_signals(signal)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl Drop for SocketEndpoint {
    fn drop(&mut self) {
        // Notify the other side that we are no longer writing
        self.write_bus.is_closed.store(true, Ordering::SeqCst);
        self.write_bus.semaphore.signal();

        // wake up any threads blocked waiting to read from the now closed bus 
        let int_state = interrupts_enabled();
        disable_interrupts();
        let mut wq = self.write_bus.read_waiters.lock();
        loop {
            let thread = wq.pop();
            if thread.is_null() {
                break;
            } else {
                wake_thread(thread);
            }
        }
        if int_state { enable_interrupts(); }
    }
}

impl SocketEndpoint {
    pub fn new_pair() -> (Arc<SocketEndpoint>, Arc<SocketEndpoint>) {
        let bus1 = Arc::new(SocketBus::new());
        let bus2 = Arc::new(SocketBus::new());

        let ep1 = Arc::new(SocketEndpoint {
            read_bus: bus1.clone(),
            write_bus: bus2.clone(),
            is_nb: AtomicBool::new(false),
        });

        let ep2 = Arc::new(SocketEndpoint {
            read_bus: bus2,
            write_bus: bus1,
            is_nb: AtomicBool::new(false),
        });

        (ep1, ep2)
    }

    fn read(&self, buffer_ptr: *mut u8, len: usize) -> Result<usize, InvocationError> {
        if len == 0 {
            return Ok(0);
        }
        loop {
            let mut has_data = false;
            let mut count = 0;
            let mut is_eof = false;

            {
                let mut bus = self.read_bus.buffer.lock();
                if !bus.is_empty() {
                    let mut temp_buf = [0u8; 512];
                    let to_read = min(len, temp_buf.len());
                    count = bus.pop_slice(&mut temp_buf[..to_read]);

                    if !safe_copy_to(buffer_ptr, temp_buf.as_ptr(), count) {
                        return Err(InvocationError::InvalidPointer);
                    }
                    has_data = true;
                } else if self.read_bus.is_closed.load(Ordering::SeqCst) {
                    is_eof = true;
                } else if self.is_nb.load(Ordering::SeqCst) {
                    return Err(InvocationError::WouldBlock);
                }
            }

            if has_data {
                if count > 0 {
                    let int_state = interrupts_enabled();
                    disable_interrupts();

                    let mut wq = self.read_bus.write_waiters.lock();
                    let thread = wq.pop();
                    drop(wq);

                    if int_state { enable_interrupts(); }

                    if !thread.is_null() {
                        wake_thread(thread);
                    }
                }
                return Ok(count);
            }

            if is_eof {
                return Ok(0);
            }

            self.read_bus.semaphore.wait();
        }
    }

    fn write(&self, buffer_ptr: *const u8, len: usize) -> Result<usize, InvocationError> {
        if self.write_bus.is_closed.load(Ordering::SeqCst) {
            return Err(InvocationError::UnsupportedOperation); // Broken pipe
        }
        if len == 0 {
            return Ok(0);
        }

        let mut temp_buf = [0u8; 512];
        let to_write = min(len, temp_buf.len());

        if !safe_copy_from(temp_buf.as_mut_ptr(), buffer_ptr, to_write) {
            return Err(InvocationError::InvalidPointer);
        }

        loop { 
            let mut wrote_data = false;
            let mut count = 0;
            let mut is_broken = false;

            {
                let mut bus = self.write_bus.buffer.lock();
                if self.write_bus.is_closed.load(Ordering::SeqCst) {
                    is_broken = true;
                } else if !bus.is_full() {
                    count = bus.push_slice(&temp_buf[..to_write]);
                    wrote_data = true;
                } else if self.is_nb.load(Ordering::SeqCst) {
                    return Err(InvocationError::WouldBlock);
                }
            }

            if is_broken {
                return Err(InvocationError::UnsupportedOperation) 
            }

            if wrote_data {
                if count > 0 {
                    self.write_bus.semaphore.signal();

                    let int_state = interrupts_enabled();
                    disable_interrupts();

                    let mut wq = self.write_bus.read_waiters.lock();
                    let thread = wq.pop();
                    drop(wq);

                    if int_state { enable_interrupts(); }

                    if !thread.is_null() {
                        wake_thread(thread);
                    }
                }
                return Ok(count);
            }
            // block bc buffer is full
            let int_state = interrupts_enabled();
            disable_interrupts();

            let sched = &mut get_core_data().scheduler;
            let thread = sched.current_thread;
            let mut wq = self.write_bus.write_waiters.lock();
            unsafe {
                (*thread).state = ThreadState::Blocked;
            }
            wq.push(thread);
            drop(wq);

            sched.schedule();

            if int_state { enable_interrupts(); }
        }
    }

    fn wait_for_signals(&self, signal: Signal) -> Result<usize, InvocationError> {
        loop {
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
                return Ok(0);
            }

            let int_state = interrupts_enabled();
            disable_interrupts();

            let sched = &mut get_core_data().scheduler;
            let thread = sched.current_thread;
            let mut wq = if is_write {
                self.write_bus.write_waiters.lock()
            } else {
                self.read_bus.read_waiters.lock()
            };

            unsafe {
                (*thread).state = ThreadState::Blocked;
            }

            wq.push(thread);
            drop(wq);

            sched.schedule();

            if int_state { enable_interrupts(); }
        }
    }
}

#[derive(Debug)]
pub struct SocketFactory {}

impl KernelObject for SocketFactory {
    fn type_name(&self) -> &'static str {
        "SocketFactory"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
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
