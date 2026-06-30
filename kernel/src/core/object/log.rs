use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{
    AtomicU64,
    Ordering,
};
use core::task::{
    Context,
    Poll,
    Waker,
};

use async_trait::async_trait;
use hal::usercopy::{
    safe_copy_from,
    safe_copy_to,
};
use vespertine_abi::{
    AccessRights,
    FileOp,
    Invocation,
    Signal,
    WaitOp,
};

use crate::core::executor::waiter::{
    AsyncWaiter,
    WaiterList,
    wake_all,
};
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::{
    KernelObject,
    ObjectWaitFuture,
};
use crate::core::sync::Mutex;
use crate::process::current_process;
use crate::time::get_realtime;
use crate::klogln;

const MAX_MESSAGE_BYTES: usize = 2048;
const MAX_RECORD_BYTES: usize = 4096;
const MAX_QUEUED_RECORDS: usize = 1024;
const FALLBACK_PROCESS: &str = "kernel";

#[derive(Debug)]
pub struct Log {
    inner: Mutex<LogInner>,
    dropped: AtomicU64,
}

#[derive(Debug)]
struct LogInner {
    records: VecDeque<Vec<u8>>,
    readers: WaiterList,
    readable_waiters: WaiterList,
}

struct LogReadFuture<'a> {
    log: &'a Log,
    buffer_ptr: usize,
    len: usize,
    waiter: alloc::sync::Arc<AsyncWaiter>,
}

impl Log {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LogInner { records: VecDeque::new(), readers: WaiterList::new(), readable_waiters: WaiterList::new() }),
            dropped: AtomicU64::new(0),
        }
    }

    fn notify_readers(&self) {
        let wakers = {
            let mut inner = self.inner.lock();
            let mut wakers = inner.readers.take_wakers();
            wakers.extend(inner.readable_waiters.take_wakers());
            wakers
        };

        wake_all(wakers);
    }

    fn enqueue(&self, record: Vec<u8>) {
        let mut inner = self.inner.lock();

        if inner.records.len() >= MAX_QUEUED_RECORDS {
            inner.records.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }

        inner.records.push_back(record);
        drop(inner);

        self.notify_readers();
    }

    fn build_record(&self, message: &[u8]) -> Vec<u8> {
        let (ts_sec, ts_nsec) = get_realtime();
        let dropped = self.dropped.load(Ordering::Relaxed);

        let (pid, user, process) = match current_process() {
            Some(process) => (process.proc_id, process.credentials.user().0, process.proc_name.as_str()),
            None => (usize::MAX, 0, FALLBACK_PROCESS),
        };

        let mut record = String::new();
        let _ = write!(record, "{{\"ts_sec\":{},\"ts_nsec\":{},\"pid\":{},\"user\":{},\"process\":", ts_sec, ts_nsec, pid, user,);
        push_json_string(&mut record, process);
        let _ = write!(record, ",\"level\":\"info\",\"dropped\":{},\"message\":", dropped);
        push_json_bytes(&mut record, message);
        let _ = record.write_char('}');
        let _ = record.write_char('\n');

        record.into_bytes()
    }
}

impl Drop for LogReadFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for LogReadFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut inner = this.log.inner.lock();

        if let Some(record) = inner.records.front() {
            if this.len < record.len() {
                return Poll::Ready(Err(InvocationError::BufferFull));
            }

            let record = inner.records.pop_front().expect("front record disappeared");
            drop(inner);

            if !safe_copy_to(this.buffer_ptr as *mut u8, record.as_ptr(), record.len()) {
                return Poll::Ready(Err(InvocationError::InvalidPointer));
            }

            return Poll::Ready(Ok(record.len()));
        }

        inner.readers.register(&this.waiter, cx.waker());
        Poll::Pending
    }
}

#[async_trait]
impl KernelObject for Log {
    fn type_name(&self) -> &'static str { "System Log" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::File(FileOp::Read { offset: _, buffer_ptr, len }) => {
                calling_rights.err_if_no(AccessRights::READ)?;

                if len == 0 {
                    return Ok(0);
                }

                LogReadFuture { log: self, buffer_ptr, len, waiter: AsyncWaiter::new() }.await
            }
            Invocation::File(FileOp::Write { offset: _, buffer_ptr, len }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;

                if len > MAX_MESSAGE_BYTES {
                    return Err(InvocationError::BufferFull);
                }

                let mut buf = [0u8; MAX_MESSAGE_BYTES];
                if !safe_copy_from(buf.as_mut_ptr(), buffer_ptr as *const u8, len) {
                    return Err(InvocationError::InvalidPointer);
                }

                if let Ok(s) = str::from_utf8(&buf[..len]) {
                    klogln!("{}", s);
                }

                let record = self.build_record(&buf[..len]);
                if record.len() > MAX_RECORD_BYTES {
                    return Err(InvocationError::BufferFull);
                }

                self.enqueue(record);

                Ok(len)
            }
            Invocation::Wait(WaitOp::One(signal)) => ObjectWaitFuture::new(self, signal).await,
            Invocation::Wait(WaitOp::Many { .. }) => Err(InvocationError::UnsupportedOperation),
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn current_signals(&self) -> Signal { if self.inner.lock().records.is_empty() { Signal(0) } else { Signal::READABLE } }

    fn register_waiter(&self, requested: Signal, waiter: &alloc::sync::Arc<AsyncWaiter>, waker: &Waker) -> Result<(), InvocationError> {
        if requested != (requested & Signal::READABLE) {
            return Err(InvocationError::UnsupportedOperation);
        }

        self.inner.lock().readable_waiters.register(waiter, waker);
        Ok(())
    }
}

fn push_json_string(dst: &mut String, value: &str) {
    let _ = dst.write_char('"');
    push_json_escaped(dst, value.as_bytes());
    let _ = dst.write_char('"');
}

fn push_json_bytes(dst: &mut String, value: &[u8]) {
    let _ = dst.write_char('"');
    push_json_escaped(dst, value);
    let _ = dst.write_char('"');
}

fn push_json_escaped(dst: &mut String, value: &[u8]) {
    for byte in value {
        match *byte {
            b'"' => {
                let _ = dst.write_str("\\\"");
            }
            b'\\' => {
                let _ = dst.write_str("\\\\");
            }
            b'\n' => {
                let _ = dst.write_str("\\n");
            }
            b'\r' => {
                let _ = dst.write_str("\\r");
            }
            b'\t' => {
                let _ = dst.write_str("\\t");
            }
            0x20..=0x7e => {
                let _ = dst.write_char(*byte as char);
            }
            _ => {
                let _ = write!(dst, "\\u{:04x}", *byte);
            }
        }
    }
}
