use alloc::boxed::Box;
use alloc::slice;
use alloc::sync::Arc;
use core::cmp;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::Ordering;
use core::task::{
    Context,
    Poll,
};

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    FileOp,
    Invocation,
    ObjectOp,
};

use crate::arch::x86_64::task::syscall::{
    safe_copy_from,
    safe_copy_to,
};
use crate::core::asynchronous::waiter::AsyncWaiter;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::socket::SocketEndpoint;
use crate::core::object::obj::KernelObject;
use crate::core::thread::get_current_process;

#[derive(Debug)]
pub struct UserObject {
    pub channel: Arc<SocketEndpoint>,
}

#[async_trait]
impl KernelObject for UserObject {
    fn type_name(&self) -> &'static str { "UserObject" }

    async fn invoke(&self, invocation: Invocation, rights: AccessRights) -> Result<usize, InvocationError> {
        let header_bytes = unsafe { slice::from_raw_parts(&invocation as *const _ as *const u8, size_of::<Invocation>()) };
        write_internal(&self.channel, header_bytes).await?;

        if let Invocation::File(FileOp::Write { offset, buffer_ptr, len }) = invocation {
            let mut total_sent = 0;
            let mut temp_buf = [0u8; 512];
            while total_sent < len {
                let to_send = cmp::min(len - total_sent, 512);
                if !safe_copy_from(temp_buf.as_mut_ptr(), (buffer_ptr + total_sent) as *const u8, to_send) {
                    return Err(InvocationError::InvalidPointer);
                }
                write_internal(&self.channel, &temp_buf[..to_send]).await?;
                total_sent += to_send;
            }
        }

        let mut status_buf = [0u8; size_of::<usize>()];
        read_internal(&self.channel, &mut status_buf).await?;
        let status = usize::from_ne_bytes(status_buf);

        if let Invocation::File(FileOp::Read { offset, buffer_ptr, len }) = invocation {
            let bytes_to_recieve = cmp::min(status, len);
            let mut total_recieved = 0;
            let mut temp_buf = [0u8; 512];

            while total_recieved < bytes_to_recieve {
                let to_read = cmp::min(bytes_to_recieve - total_recieved, 512);
                read_internal(&self.channel, &mut temp_buf[..to_read]).await?;

                if !safe_copy_to((buffer_ptr + total_recieved) as *mut u8, temp_buf.as_ptr(), to_read) {
                    return Err(InvocationError::InvalidPointer);
                }
                total_recieved += to_read;
            }
            return Ok(total_recieved);
        }
        Ok(status)
    }
}

pub struct InternalWriteFuture<'a> {
    channel: &'a SocketEndpoint,
    data: &'a [u8],
    waiter: Arc<AsyncWaiter>,
}

impl Drop for InternalWriteFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for InternalWriteFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.channel.write_bus.is_closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(InvocationError::UnsupportedOperation));
        }

        let mut inner = this.channel.write_bus.inner.lock();
        if inner.buffer.is_full() {
            inner.write_waiters.register(&this.waiter, cx.waker());
            return Poll::Pending;
        }

        let count = inner.buffer.push_slice(this.data);
        drop(inner);
        this.channel.write_bus.notify_readable();
        Poll::Ready(Ok(count))
    }
}

pub struct InternalReadFuture<'a> {
    channel: &'a SocketEndpoint,
    data: &'a mut [u8],
    waiter: Arc<AsyncWaiter>,
}

impl Drop for InternalReadFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for InternalReadFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut inner = this.channel.read_bus.inner.lock();
        if !inner.buffer.is_empty() {
            let count = inner.buffer.pop_slice(this.data);
            drop(inner);
            this.channel.read_bus.notify_writable();
            return Poll::Ready(Ok(count));
        }

        if this.channel.read_bus.is_closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(0));
        }

        inner.read_waiters.register(&this.waiter, cx.waker());
        Poll::Pending
    }
}

pub async fn write_internal(channel: &Arc<SocketEndpoint>, data: &[u8]) -> Result<(), InvocationError> {
    let mut sent = 0;
    while sent < data.len() {
        let to_send = cmp::min(data.len() - sent, 512);
        let bytes = InternalWriteFuture { channel, data: &data[sent..sent + to_send], waiter: AsyncWaiter::new() }.await?;
        sent += bytes;
    }
    Ok(())
}

pub async fn read_internal(channel: &Arc<SocketEndpoint>, data: &mut [u8]) -> Result<(), InvocationError> {
    let mut received = 0;
    while received < data.len() {
        let to_read = cmp::min(data.len() - received, 512);
        let bytes = InternalReadFuture { channel, data: &mut data[received..received + to_read], waiter: AsyncWaiter::new() }.await?;

        if bytes == 0 {
            break;
        } // EOF reached during read
        received += bytes;
    }
    Ok(())
}

#[derive(Debug)]
pub struct ObjectFactory {}

#[async_trait]
impl KernelObject for ObjectFactory {
    fn type_name(&self) -> &'static str { "ObjectFactory" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Object(ObjectOp::CreateProxy { socket }) => {
                if !calling_rights.contains(AccessRights::CREATE) {
                    return Err(InvocationError::AccessDenied);
                }

                let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;

                let table = caller.proc_handles.read();
                let entry = table.resolve_entry(socket, AccessRights::READ | AccessRights::WRITE)?;

                if entry.object.type_name() != "Socket" {
                    return Err(InvocationError::InvalidArgument);
                }

                // downcast Arc<dyn KernelObject> to Arc<SocketEndpoint> safely
                let socket = unsafe {
                    let raw_fat = Arc::into_raw(entry.object.clone());
                    let raw_thin = raw_fat as *const () as *const SocketEndpoint;
                    Arc::from_raw(raw_thin)
                };

                let proxy = Arc::new(UserObject { channel: socket });
                let handle = caller.proc_handles.write().insert(proxy, AccessRights::all());
                Ok(handle.0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
