use core::cmp::min;
use core::sync::atomic::{AtomicBool, Ordering};
use alloc::sync::Arc;
use crate::core::sync::{Mutex, Semaphore};
use crate::core::object::obj::KernelObject;
use crate::core::object::invoke::InvocationError;
use vespertine_abi::{Invocation, HandleID};
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
}

impl SocketBus {
    pub fn new() -> Self {
        Self {
            buffer: Mutex::new(RingBuffer::new()),
            semaphore: Semaphore::new(0),
            is_closed: AtomicBool::new(false),
        }
    }
}

#[derive(Debug)]
pub struct SocketEndpoint {
    pub read_bus: Arc<SocketBus>,
    pub write_bus: Arc<SocketBus>,
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
            }
            Invocation::File(FileOp::Write { buffer_ptr, len, .. }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                self.write(buffer_ptr, len)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

impl Drop for SocketEndpoint {
    fn drop(&mut self) {
        // Notify the other side that we are no longer writing
        self.write_bus.is_closed.store(true, Ordering::SeqCst);
        self.write_bus.semaphore.signal();
    }
}

impl SocketEndpoint {
    pub fn new_pair() -> (Arc<SocketEndpoint>, Arc<SocketEndpoint>) {
        let bus1 = Arc::new(SocketBus::new());
        let bus2 = Arc::new(SocketBus::new());

        let ep1 = Arc::new(SocketEndpoint {
            read_bus: bus1.clone(),
            write_bus: bus2.clone(),
        });

        let ep2 = Arc::new(SocketEndpoint {
            read_bus: bus2,
            write_bus: bus1,
        });

        (ep1, ep2)
    }

    fn read(&self, buffer_ptr: *mut u8, len: usize) -> Result<usize, InvocationError> {
        if len == 0 {
            return Ok(0);
        }
        loop {
            {
                let mut bus = self.read_bus.buffer.lock();
                if !bus.is_empty() {
                    let mut temp_buf = [0u8; 512];
                    let to_read = min(len, temp_buf.len());
                    let read_count = bus.pop_slice(&mut temp_buf[..to_read]);

                    if !safe_copy_to(buffer_ptr, temp_buf.as_ptr(), read_count) {
                        return Err(InvocationError::InvalidArgument);
                    }
                    return Ok(read_count);
                }
                if self.read_bus.is_closed.load(Ordering::SeqCst) {
                    return Ok(0); // EOF
                }
            }
            self.read_bus.semaphore.wait();
        }
    }

    fn write(&self, buffer_ptr: *const u8, len: usize) -> Result<usize, InvocationError> {
        if self.write_bus.is_closed.load(Ordering::SeqCst) {
            return Err(InvocationError::UnsupportedOperation); // Broken pipe
        }

        let mut temp_buf = [0u8; 512];
        let to_write = min(len, temp_buf.len());

        if !safe_copy_from(temp_buf.as_mut_ptr(), buffer_ptr, to_write) {
            return Err(InvocationError::InvalidArgument);
        }

        let write_count = {
            let mut bus = self.write_bus.buffer.lock();
            bus.push_slice(&temp_buf[..to_write])
        };

        if write_count > 0 {
            self.write_bus.semaphore.signal();
        }

        Ok(write_count)
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
