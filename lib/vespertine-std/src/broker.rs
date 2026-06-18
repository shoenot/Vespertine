use vespertine_abi::{AccessRights, BrokerOp, CapabilityID, HandleID, Invocation};
use vespertine_rt::syscall::{sys_close, sys_invoke};

use crate::Error;

pub struct Broker {
    handle: HandleID,
}

impl Broker {
    pub fn from_handle(handle: HandleID) -> Self {
        Self { handle }
    }

    pub fn request(
        &self,
        capability: CapabilityID,
        requested_rights: AccessRights,
    ) -> Result<HandleID, Error> {
        let op = Invocation::Broker(BrokerOp::Request {
            capability,
            requested_rights,
        });
        let handle = sys_invoke(self.handle, &op).map_err(Error::from)?;
        Ok(HandleID(handle))
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}
