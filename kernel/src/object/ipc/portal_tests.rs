use alloc::task::Wake;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll};

use alloc::sync::Arc;
use vespertine_abi::{AccessRights, BrokerOp, CapabilityID, Invocation};

use super::{Portal, SocketEndpoint, current_process};
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::object::obj::KernelObject;
use crate::executor::waiter::{TerminationToken, wake_all};

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn assert_connect_rolls_back(drop_reader: bool, expected: InvocationError) {
    let caller = current_process().expect("test process");
    let (reader, writer) = SocketEndpoint::new_pair();
    let bus = Arc::clone(&writer.write_bus);
    let queued = bus.inner.lock().buffer.available_space() - 4;
    let filler = alloc::vec![0xa5; queued];
    assert_eq!(bus.inner.lock().buffer.push_slice(&filler), queued);

    let portal = Portal {
        owner: Arc::clone(caller),
        accept_tx: writer,
        capability: CapabilityID(1),
        max_rights: AccessRights::READ,
    };
    let token = TerminationToken::new();
    let initial_handles = caller.handles.read().entries.len();
    let mut connect = portal.invoke(
        Invocation::Broker(BrokerOp::Request {
            capability: portal.capability,
            requested_rights: AccessRights::READ,
        }),
        AccessRights::READ,
        InvokeContext::for_thread(&token),
    );
    let counter = Arc::new(WakeCount::default());
    let waker = Arc::clone(&counter).into();
    let mut cx = Context::from_waker(&waker);

    assert_eq!(connect.as_mut().poll(&mut cx), Poll::Pending);
    assert_eq!(caller.handles.read().entries.len(), initial_handles + 3);
    assert_eq!(bus.inner.lock().buffer.len(), queued);

    let reader = if drop_reader {
        drop(reader);
        None
    } else {
        wake_all(token.request_and_take_wakers());
        Some(reader)
    };

    assert_eq!(counter.0.load(Ordering::Relaxed), 1);
    assert_eq!(connect.as_mut().poll(&mut cx), Poll::Ready(Err(expected)));
    assert_eq!(caller.handles.read().entries.len(), initial_handles);
    let mut remaining = alloc::vec![0; queued + 8];
    assert_eq!(bus.inner.lock().buffer.pop_slice(&mut remaining), queued);
    assert_eq!(&remaining[..queued], filler.as_slice());
    drop(reader);
}

pub(super) fn run() {
    crate::klogln!(Info, "Test/Portal", "Blocked connect rolls back on kill and accept-reader closure");
    assert_connect_rolls_back(false, InvocationError::Terminating);
    assert_connect_rolls_back(true, InvocationError::UnsupportedOperation);
}
