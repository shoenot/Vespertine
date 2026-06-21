use vespertine_abi::{AccessRights, CapabilityID, HandleID, Invocation, PortalOp, tag::CAP_PORTAL_FACTORY};
use vespertine_rt::syscall::{sys_close, sys_invoke};

use crate::{Error, broker::Broker, fs::{Path, resolve}};

pub struct PortalFactory {
    handle: HandleID,
}

impl PortalFactory {
    pub fn request() -> Result<Self, Error> {
        let broker_handle = resolve(
            &Path::new("/System/Services/Portal"), 
            AccessRights::READ
        )?;
        let broker = Broker::from_handle(broker_handle);
        let handle = broker.request(CAP_PORTAL_FACTORY, AccessRights::CREATE)?;
        Ok(Self { handle })
    }

    pub fn create(&self, cap: CapabilityID, max_rights: AccessRights) -> Result<(HandleID, HandleID), Error> {
        let op = PortalOp::Create { capability: cap, max_rights };
        let packed = sys_invoke(
            self.handle, 
            &Invocation::Portal(op),
        ).map_err(Error::from)?;
        let portal = HandleID(packed & 0xffff_ffff);
        let accept = HandleID((packed >> 32) & 0xffff_ffff);
        Ok((portal, accept))
    }
}

impl Drop for PortalFactory {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}

pub type PortalOfferId = usize;

pub fn offer_handle(session: HandleID, handle: HandleID, max_rights: AccessRights) -> Result<PortalOfferId, Error> {
    sys_invoke(
        session, 
        &Invocation::Portal(PortalOp::Offer { handle, max_rights })
    ).map_err(Error::from)
}

pub fn accept_handle(session: HandleID, offer_id: PortalOfferId, requested_rights: AccessRights) -> Result<HandleID, Error> {
    let handle = sys_invoke(
        session,
        &Invocation::Portal(PortalOp::Accept { offer_id, requested_rights }),
    ).map_err(Error::from)?;
    Ok(HandleID(handle))
}

pub fn revoke_offer(session: HandleID, offer_id: PortalOfferId) -> Result<(), Error> {
    sys_invoke(
        session,
        &Invocation::Portal(PortalOp::Revoke { offer_id }),
    ).map(|_| ()).map_err(Error::from)
}
