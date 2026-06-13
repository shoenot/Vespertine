use alloc::boxed::Box;
use alloc::sync::Arc;
use core::{fmt::Debug, pin::Pin, task::{Context, Poll, Waker}};

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    Invocation, Signal,
};

use crate::core::{asynchronous::waiter::AsyncWaiter, object::invoke::InvocationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Directory,
    File, 
    Other,
}

#[async_trait]
pub trait KernelObject: Send + Sync + Debug {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError>;

    fn type_name(&self) -> &'static str { "Unknown" }

    fn as_any(&self) -> &dyn core::any::Any { &() }

    fn current_signals(&self) -> Signal { Signal(0) }

    fn register_waiter(&self, _requested: Signal, _waiter: &Arc<AsyncWaiter>, _waker: &Waker) -> Result<(), InvocationError> { 
        Err(InvocationError::UnsupportedOperation) 
    }

    fn object_type(&self) -> ObjectType { ObjectType::Other }
}

pub fn matching_signals(current: Signal, requested: Signal) -> Signal {
    current & requested
}

pub struct ObjectWaitFuture<'a> {
    object: &'a dyn KernelObject,
    requested: Signal,
    waiter: Arc<AsyncWaiter>,
}

impl<'a> ObjectWaitFuture<'a> {
    pub fn new(object: &'a dyn KernelObject, requested: Signal) -> Self {
        Self {
            object,
            requested,
            waiter: AsyncWaiter::new(),
        }
    }
}

impl Drop for ObjectWaitFuture<'_> {
    fn drop(&mut self) {
        self.waiter.deactivate();
    }
}

impl Future for ObjectWaitFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if matching_signals(this.object.current_signals(), this.requested) != Signal(0) {
            return Poll::Ready(Ok(0));
        }

        if let Err(error) =
            this.object
                .register_waiter(this.requested, &this.waiter, cx.waker())
        {
            return Poll::Ready(Err(error));
        }

        if matching_signals(this.object.current_signals(), this.requested) != Signal(0) {
            this.waiter.deactivate();
            Poll::Ready(Ok(0))
        } else {
            Poll::Pending
        }
    }
}


#[derive(Debug)]
pub struct HandleEntry {
    pub rights: AccessRights,
    pub object: Arc<dyn KernelObject>,
}
