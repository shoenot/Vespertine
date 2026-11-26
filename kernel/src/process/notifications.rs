use core::task::Waker;
use alloc::{boxed::Box, vec::Vec};

use alloc::sync::Arc;
use async_trait::async_trait;
use vespertine_abi::{AccessRights, HandleID, Invocation, NotificationOp, Signal, WaitOp};

use crate::{executor::waiter::{AsyncWaiter, WaiterList}, object::{help::RightsWrapper, invoke::InvocationError, obj::{KernelObject, ObjectWaitFuture}}, sync::TicketLock as IrqTicketLock};

#[derive(Debug, Clone, Copy)]
enum NotificationState {
    Empty, 
    Pending { sender_pid: usize },
    Released,
}

#[derive(Debug)]
struct ReceiverState {
    notification: NotificationState,
    waiters: WaiterList,
}

#[derive(Debug)]
pub struct NotificationReceiver { 
    state: IrqTicketLock<ReceiverState>,
}

#[derive(Debug)]
pub struct NotificationClaim {
    pub(super) owner_handle: HandleID,
    pub(super) receiver: Arc<NotificationReceiver>,
}

impl NotificationReceiver {
    pub fn new() -> Self {
        Self { state: IrqTicketLock::new(
            ReceiverState {
                notification: NotificationState::Empty,
                waiters: WaiterList::new(),
            }
        )}
    }

    fn take(&self) -> Result<usize, InvocationError> {
        let mut state = self.state.lock();
        match state.notification {
            NotificationState::Pending { sender_pid } => {
                state.notification = NotificationState::Empty;
                Ok(sender_pid)
            },
            NotificationState::Empty => Err(InvocationError::WouldBlock),
            NotificationState::Released => Err(InvocationError::InvalidHandle),
        }
    }

    pub(super) fn release(&self) -> Vec<Waker> {
        let mut state = self.state.lock();
        state.notification = NotificationState::Released;
        state.waiters.take_wakers()
    }

    pub(super) fn post(&self, sender_pid: usize) -> Vec<Waker> {
        let mut state = self.state.lock();
        match state.notification {
            NotificationState::Empty => {
                state.notification = NotificationState::Pending { sender_pid };
                state.waiters.take_wakers()
            },
            NotificationState::Pending { .. } => Vec::new(),
            NotificationState::Released => unreachable!("Active claim has been released"),
        }
    }
}

#[async_trait]
impl KernelObject for NotificationReceiver {
    fn type_name(&self) -> &'static str {
        "NotificationReceiver"
    }

    fn current_signals(&self) -> Signal {
        match self.state.lock().notification {
            NotificationState::Empty => Signal(0),
            NotificationState::Pending { .. } | NotificationState::Released => {
                Signal::READABLE
            }
        }
    }

    fn register_waiter(&self, requested: Signal, waiter: &Arc<AsyncWaiter>, waker: &Waker) -> Result<(), InvocationError> {
        if requested != Signal::READABLE { return Err(InvocationError::UnsupportedOperation) };
        self.state.lock().waiters.register(waiter, waker);
        Ok(())
    }

    async fn invoke(&self, invocation: Invocation, rights: AccessRights) -> Result<usize, InvocationError> {
        rights.err_if_no(AccessRights::READ)?;
        match invocation {
            Invocation::Notification(NotificationOp::TakeNotification) => self.take(),
            Invocation::Wait(WaitOp::One(signal)) if signal == Signal::READABLE => {
                ObjectWaitFuture::new(self, signal).await
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
