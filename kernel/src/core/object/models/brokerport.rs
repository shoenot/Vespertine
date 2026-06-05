use core::{future::poll_fn, task::{Poll, Waker}};

use alloc::{collections::vec_deque::VecDeque, sync::Arc};
use alloc::boxed::Box;
use async_trait::async_trait;
use vespertine_abi::{AccessRights, BrokerOp, Invocation};

use crate::core::{object::{invoke::InvocationError, models::{process::ProcessControlBlock, socket::SocketEndpoint}, obj::KernelObject}, sync::Mutex, thread::get_current_process};



#[derive(Debug)]
pub struct ResourceBrokerPort {
    queue: Mutex<VecDeque<(Arc<SocketEndpoint>, Arc<ProcessControlBlock>)>>,
    waker: Mutex<Option<Waker>>,
}

impl ResourceBrokerPort {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            waker: Mutex::new(None),
        }
    }
}

#[async_trait]
impl KernelObject for ResourceBrokerPort {
    fn type_name(&self) ->  &'static str {
        "ResourceBroker"
    }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Broker(BrokerOp::Connect { socket_to_give }) => {
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }

                let (ep_client, ep_broker) = SocketEndpoint::new_pair();
                let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let client_handle = caller.proc_handles.write().insert(ep_client, AccessRights::all());

                self.queue.lock().push_back((ep_broker, caller.clone()));

                if let Some(waker) = self.waker.lock().take() {
                    waker.wake();
                }

                Ok(client_handle.0)
            }, 
            Invocation::Broker(BrokerOp::Accept) => {
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }

                let (ep, client_proc) = poll_fn(|cx| {
                    let mut queue = self.queue.lock();
                    if let Some(ep) = queue.pop_front() {
                        Poll::Ready(ep)
                    } else {
                        *self.waker.lock() = Some(cx.waker().clone());
                        Poll::Pending
                    }
                }).await;  

                let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;

                let ep_handle = caller.proc_handles.write().insert(ep, AccessRights::all());
                let proc_handle = caller.proc_handles.write().insert(client_proc, AccessRights::all());

                let ret = (ep_handle.0 & 0xFFFF_FFFF) | ((proc_handle.0 & 0xFFFF_FFFF) << 32);
                Ok(ret)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
} 
