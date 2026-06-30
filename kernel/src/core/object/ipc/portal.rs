use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use core::task::Waker;

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    BrokerOp,
    CapabilityID,
    HandleID,
    Invocation,
    PortalOp,
    Signal,
};

use crate::executor::waiter::AsyncWaiter;
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::process::Process;
use crate::core::object::ipc::socket::SocketEndpoint;
use crate::core::object::obj::KernelObject;
use crate::sync::Mutex;
use crate::process::current_process;

const MAX_SESSION_OFFERS: usize = 64;

#[derive(Debug)]
pub struct Portal {
    owner: Process,
    accept_tx: Arc<SocketEndpoint>,
    capability: CapabilityID,
    max_rights: AccessRights,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum PortalSessionRole {
    Client,
    Server,
}

#[derive(Debug)]
struct OfferedHandle {
    object: Arc<dyn KernelObject>,
    max_rights: AccessRights,
}

#[derive(Debug)]
struct OfferTable {
    next_id: usize,
    entries: BTreeMap<usize, OfferedHandle>,
}

impl OfferTable {
    fn new() -> Self { Self { next_id: 1, entries: BTreeMap::new() } }
}

#[derive(Debug)]
struct PortalSessionShared {
    client: Process,
    server: Process,
    client_offers: Mutex<OfferTable>,
    server_offers: Mutex<OfferTable>,
}

#[derive(Debug)]
struct PortalSession {
    role: PortalSessionRole,
    endpoint: Arc<SocketEndpoint>,
    shared: Arc<PortalSessionShared>,
}

impl PortalSession {
    fn new_pair(ce: Arc<SocketEndpoint>, se: Arc<SocketEndpoint>, c: Process, s: Process) -> (Arc<Self>, Arc<Self>) {
        let shared = Arc::new(PortalSessionShared {
            client: c,
            server: s,
            client_offers: Mutex::new(OfferTable::new()),
            server_offers: Mutex::new(OfferTable::new()),
        });
        let client_session = Arc::new(Self { role: PortalSessionRole::Client, endpoint: ce, shared: shared.clone() });
        let server_session = Arc::new(Self { role: PortalSessionRole::Server, endpoint: se, shared });
        (client_session, server_session)
    }

    fn expected_process(&self) -> &Process {
        match self.role {
            PortalSessionRole::Client => &self.shared.client,
            PortalSessionRole::Server => &self.shared.server,
        }
    }

    fn current_process(&self) -> Result<Process, InvocationError> {
        let process = current_process().ok_or(InvocationError::InvalidHandle)?;

        if !Arc::ptr_eq(&process, self.expected_process()) {
            return Err(InvocationError::AccessDenied);
        }

        Ok(process.clone())
    }

    fn outgoing_offers(&self) -> &Mutex<OfferTable> {
        match self.role {
            PortalSessionRole::Client => &self.shared.client_offers,
            PortalSessionRole::Server => &self.shared.server_offers,
        }
    }

    fn incoming_offers(&self) -> &Mutex<OfferTable> {
        match self.role {
            PortalSessionRole::Client => &self.shared.server_offers,
            PortalSessionRole::Server => &self.shared.client_offers,
        }
    }

    fn offer(&self, handle: HandleID, max_rights: AccessRights) -> Result<usize, InvocationError> {
        if max_rights == AccessRights::new() {
            return Err(InvocationError::InvalidArgument);
        }

        let process = self.current_process()?;

        // resolve and pin the exact object now. closing or
        // reusing the original handle cannot change the offer.
        let object = {
            let handles = process.handles.read();
            let entry = handles.resolve_entry(handle, max_rights)?;
            entry.object.clone()
        };

        let mut offers = self.outgoing_offers().lock();

        if offers.entries.len() >= MAX_SESSION_OFFERS {
            return Err(InvocationError::BufferFull);
        }

        let offer_id = offers.next_id;

        offers.next_id = offers.next_id.checked_add(1).ok_or(InvocationError::OutOfMemory)?;

        offers.entries.insert(offer_id, OfferedHandle { object, max_rights });

        Ok(offer_id)
    }

    fn accept(&self, offer_id: usize, requested_rights: AccessRights) -> Result<usize, InvocationError> {
        if requested_rights == AccessRights::new() {
            return Err(InvocationError::InvalidArgument);
        }

        let process = self.current_process()?;

        let offered = {
            let mut offers = self.incoming_offers().lock();
            let offered = offers.entries.get(&offer_id).ok_or(InvocationError::InvalidArgument)?;

            if !offered.max_rights.contains(requested_rights) {
                return Err(InvocationError::AccessDenied);
            }

            offers.entries.remove(&offer_id).ok_or(InvocationError::InvalidArgument)?
        };

        let handle = process.handles.write().insert(offered.object, requested_rights);

        Ok(handle.0)
    }

    fn revoke(&self, offer_id: usize) -> Result<usize, InvocationError> {
        self.current_process()?;
        self.outgoing_offers().lock().entries.remove(&offer_id).ok_or(InvocationError::InvalidArgument)?;
        Ok(0)
    }
}

#[async_trait]
impl KernelObject for Portal {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        calling_rights.err_if_no(AccessRights::READ)?;
        let Invocation::Broker(BrokerOp::Request { capability, requested_rights }) = invocation else {
            return Err(InvocationError::UnsupportedOperation);
        };

        if capability != self.capability {
            return Err(InvocationError::InvalidArgument);
        }

        let granted_rights = requested_rights & self.max_rights;
        if granted_rights == AccessRights::new() {
            return Err(InvocationError::AccessDenied);
        }

        let caller = current_process().ok_or(InvocationError::InvalidHandle)?;
        let (client_endpoint, server_endpoint) = SocketEndpoint::new_pair();
        let (client_session, server_session) =
            PortalSession::new_pair(client_endpoint, server_endpoint, caller.clone(), self.owner.clone());
        let client_handle = caller.handles.write().insert(client_session, granted_rights);
        let server_handle = self.owner.handles.write().insert(server_session, AccessRights::READ | AccessRights::WRITE);

        let caller_process_handle = self.owner.handles.write().insert(caller.clone(), AccessRights::READ);

        if let Err(error) = write_accept_message(&self.accept_tx, server_handle, caller_process_handle).await {
            let _ = caller.handles.write().close(client_handle);
            let _ = self.owner.handles.write().close(server_handle);
            return Err(error);
        }
        Ok(client_handle.0)
    }
}

#[derive(Debug)]
pub struct PortalFactory;

#[async_trait]
impl KernelObject for PortalFactory {
    fn type_name(&self) -> &'static str { "PortalFactory" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        calling_rights.err_if_no(AccessRights::CREATE)?;
        let Invocation::Portal(PortalOp::Create { capability, max_rights }) = invocation else {
            return Err(InvocationError::UnsupportedOperation);
        };

        let owner = current_process().ok_or(InvocationError::InvalidHandle)?;

        let (accept_rx, accept_tx) = SocketEndpoint::new_pair();

        let portal = Arc::new(Portal { owner: owner.clone(), accept_tx, capability, max_rights });

        let mut handles = owner.handles.write();
        let portal_handle = handles.insert(portal, AccessRights::READ);
        let accept_handle = handles.insert(accept_rx, AccessRights::READ);
        Ok((portal_handle.0 & 0xffff_ffff) | ((accept_handle.0 & 0xffff_ffff) << 32))
    }
}

async fn write_accept_message(socket: &SocketEndpoint, session: HandleID, caller_process: HandleID) -> Result<(), InvocationError> {
    let mut bytes = [0u8; 8];

    bytes[0..4].copy_from_slice(&(session.0 as u32).to_le_bytes());
    bytes[4..8].copy_from_slice(&(caller_process.0 as u32).to_le_bytes());

    socket.write_all_internal(&bytes).await
}

#[async_trait]
impl KernelObject for PortalSession {
    fn type_name(&self) -> &'static str { "PortalSession" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Portal(PortalOp::Offer { handle, max_rights }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.offer(handle, max_rights)
            }
            Invocation::Portal(PortalOp::Accept { offer_id, requested_rights }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.accept(offer_id, requested_rights)
            }
            Invocation::Portal(PortalOp::Revoke { offer_id }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.revoke(offer_id)
            }
            Invocation::Portal(PortalOp::Create { .. }) => Err(InvocationError::UnsupportedOperation),
            other => self.endpoint.invoke(other, calling_rights).await,
        }
    }

    fn current_signals(&self) -> Signal { self.endpoint.current_signals() }

    fn register_waiter(&self, requested: Signal, waiter: &Arc<AsyncWaiter>, waker: &Waker) -> Result<(), InvocationError> {
        KernelObject::register_waiter(self.endpoint.as_ref(), requested, waiter, waker)
    }
}
