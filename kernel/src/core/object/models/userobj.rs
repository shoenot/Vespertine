use core::{cmp, future::poll_fn, sync::atomic::Ordering};

use alloc::{slice, sync::Arc};
use alloc::boxed::Box;
use async_trait::async_trait;
use vespertine_abi::{AccessRights, FileOp, Invocation, ObjectOp};

use crate::core::thread::get_current_process;
use crate::{arch::x86_64::task::syscall::{safe_copy_from, safe_copy_to}, core::object::{invoke::InvocationError, models::socket::SocketEndpoint, obj::KernelObject}};

#[derive(Debug)]
pub struct UserObject {
    pub channel:Arc<SocketEndpoint>,
}

#[async_trait]
impl KernelObject for UserObject {
    fn type_name(&self) ->  &'static str {
        "UserObject"
    }

    async fn invoke(&self, invocation: Invocation, rights: AccessRights) -> Result<usize, InvocationError> {
        let header_bytes = unsafe {
            slice::from_raw_parts(&invocation as *const _ as *const u8, size_of::<Invocation>())
        };
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


async fn write_internal(channel: &Arc<SocketEndpoint>, data: &[u8]) -> Result<(), InvocationError> {
    let mut sent = 0;
    while sent < data.len() {
        let bytes = poll_fn(|cx| {
            if channel.write_bus.is_closed.load(Ordering::SeqCst) {
                return core::task::Poll::Ready(Err(InvocationError::UnsupportedOperation));
            }
            let mut bus = channel.write_bus.buffer.lock();
            if bus.is_full() {
                *channel.write_bus.write_waker.lock() = Some(cx.waker().clone());
                return core::task::Poll::Pending;
            }

            let to_send = cmp::min(data.len() - sent, 512);
            let count = bus.push_slice(&data[sent..sent + to_send]);

            if let Some(waker) = channel.write_bus.read_waker.lock().take() {
                waker.wake();
            }
            core::task::Poll::Ready(Ok(count))
        }).await?;
        sent += bytes;
    }
    Ok(())
}

async fn read_internal(channel: &Arc<SocketEndpoint>, data: &mut [u8]) -> Result<(), InvocationError> {
    let mut received = 0;
    while received < data.len() {
        let bytes = poll_fn(|cx| -> core::task::Poll<Result<usize, InvocationError>> {
            let mut bus = channel.read_bus.buffer.lock();
            if !bus.is_empty() {
                let to_read = cmp::min(data.len() - received, 512);
                let count = bus.pop_slice(&mut data[received..received + to_read]);

                if let Some(waker) = channel.read_bus.write_waker.lock().take() {
                    waker.wake();
                }
                return core::task::Poll::Ready(Ok(count));
            }

            if channel.read_bus.is_closed.load(core::sync::atomic::Ordering::SeqCst) {
                return core::task::Poll::Ready(Ok(0)); // EOF
            }

            *channel.read_bus.read_waker.lock() = Some(cx.waker().clone());
            core::task::Poll::Pending
        }).await?;

        if bytes == 0 { break; } // EOF reached during read
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
