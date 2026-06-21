use core::cmp;

use alloc::sync::Arc;
use alloc::boxed::Box;
use async_trait::async_trait;
use vespertine_abi::{AccessRights, BrokerOp, CapabilityID, HandleID, Invocation, PortalOp};

use crate::{core::{object::{invoke::InvocationError, models::{process::Process, socket::SocketEndpoint, userobj::write_internal}, obj::KernelObject}, thread::get_current_process}};

#[derive(Debug)]
pub struct Portal {
    owner: Process,
    accept_tx: Arc<SocketEndpoint>,
    capability: CapabilityID,
    max_rights: AccessRights,
}

#[async_trait]
impl KernelObject for Portal {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        if !calling_rights.contains(AccessRights::READ) {
            return Err(InvocationError::AccessDenied);
        }

        let Invocation::Broker(BrokerOp::Request { capability, requested_rights }) = invocation else {
            return Err(InvocationError::UnsupportedOperation);
        };

        if capability != self.capability {
            return Err(InvocationError::InvalidArgument);
        }

        let granted_rights = requested_rights &self.max_rights;
        if granted_rights == AccessRights::new() {
            return Err(InvocationError::AccessDenied);
        }

        let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;
        let (client_ep, server_ep) = SocketEndpoint::new_pair();
        let client_handle = caller.proc_handles.write().insert(client_ep, granted_rights);
        let server_handle = self.owner.proc_handles.write().insert(server_ep, AccessRights::READ | AccessRights::WRITE);

        write_accept_message(&self.accept_tx, server_handle).await?;
        Ok(client_handle.0)
    }
}

#[derive(Debug)]
pub struct PortalFactory;

#[async_trait]
impl KernelObject for PortalFactory {
    fn type_name(&self) -> &'static str {
        "PortalFactory"
    }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        if !calling_rights.contains(AccessRights::CREATE) {
            return Err(InvocationError::AccessDenied);
        }

        let Invocation::Portal(PortalOp::Create { capability, max_rights }) = invocation else {
            return Err(InvocationError::UnsupportedOperation);
        };

        let owner = get_current_process().ok_or(InvocationError::InvalidHandle)?;

        let (accept_rx, accept_tx) = SocketEndpoint::new_pair();

        let portal = Arc::new(Portal {
            owner: owner.clone(),
            accept_tx,
            capability,
            max_rights,
        });

        let mut handles = owner.proc_handles.write();
        let portal_handle = handles.insert(portal, AccessRights::READ);
        let accept_handle = handles.insert(accept_rx, AccessRights::READ);
        Ok((portal_handle.0 & 0xffff_ffff) | ((accept_handle.0 & 0xffff_ffff) << 32))
    }
}

async fn write_accept_message(socket: &Arc<SocketEndpoint>, handle: HandleID) -> Result<(), InvocationError> {
    let raw = handle.0 as u32;
    let bytes = raw.to_le_bytes();
    write_internal(socket, &bytes).await
}
