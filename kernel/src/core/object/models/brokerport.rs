use alloc::boxed::Box;
use alloc::collections::vec_deque::VecDeque;
use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::task::{
    Context,
    Poll,
};

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    BrokerOp,
    Invocation,
};

use crate::core::asynchronous::waiter::{
    AsyncWaiter,
    WaiterList,
    wake_all,
};
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::process::ProcessControlBlock;
use crate::core::object::models::socket::SocketEndpoint;
use crate::core::object::obj::KernelObject;
use crate::core::sync::Mutex;
use crate::core::thread::get_current_process;

#[derive(Debug)]
pub struct ResourceBrokerPort {
    inner: Mutex<BrokerInner>,
}

impl ResourceBrokerPort {
    pub fn new() -> Self {
        Self { inner: Mutex::new(BrokerInner { queue: VecDeque::new(), waiters: WaiterList::new() }) }
    }
}

#[derive(Debug)]
struct BrokerInner {
    queue: VecDeque<(Arc<SocketEndpoint>, Arc<ProcessControlBlock>)>,
    waiters: WaiterList,
}

struct BrokerAcceptFuture<'a> {
    broker: &'a ResourceBrokerPort,
    waiter: Arc<AsyncWaiter>,
}

impl Drop for BrokerAcceptFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl Future for BrokerAcceptFuture<'_> {
    type Output = (Arc<SocketEndpoint>, Arc<ProcessControlBlock>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut inner = this.broker.inner.lock();

        if let Some(item) = inner.queue.pop_front() {
            return Poll::Ready(item);
        }

        inner.waiters.register(&this.waiter, cx.waker());
        Poll::Pending
    }
}

#[async_trait]
impl KernelObject for ResourceBrokerPort {
    fn type_name(&self) -> &'static str { "ResourceBroker" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Broker(BrokerOp::Connect { socket_to_give }) => {
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }

                let (ep_client, ep_broker) = SocketEndpoint::new_pair();
                let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let client_handle = caller.proc_handles.write().insert(ep_client, AccessRights::all());

                let wakers = {
                    let mut inner = self.inner.lock();
                    inner.queue.push_back((ep_broker, caller.clone()));
                    inner.waiters.take_wakers()
                };
                wake_all(wakers);

                Ok(client_handle.0)
            }
            Invocation::Broker(BrokerOp::Accept) => {
                if !calling_rights.contains(AccessRights::READ) {
                    return Err(InvocationError::AccessDenied);
                }

                let (ep, client_proc) = BrokerAcceptFuture { broker: self, waiter: AsyncWaiter::new() }.await;

                let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;

                let ep_handle = caller.proc_handles.write().insert(ep, AccessRights::all());
                let proc_handle = caller.proc_handles.write().insert(client_proc, AccessRights::all());

                let ret = (ep_handle.0 & 0xFFFF_FFFF) | ((proc_handle.0 & 0xFFFF_FFFF) << 32);
                Ok(ret)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
