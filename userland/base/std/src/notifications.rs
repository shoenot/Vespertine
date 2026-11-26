use vespertine_abi::{HandleID, Signal, SysError};
use vrt::syscall::{sys_claim_notifications, sys_close, sys_try_take_notifications, sys_wait};

use crate::{Error, env};



pub struct Notifications {
    handle: HandleID,
}

impl Notifications {
    pub fn claim() -> Result<Self, Error> {
        let handle = sys_claim_notifications(env::self_handle())?;
        Ok(Self { handle })
    }

    pub fn handle(&self) -> HandleID {
        self.handle
    }

    pub fn try_take(&self) -> Result<Option<usize>, Error> {
        match sys_try_take_notifications(self.handle) {
            Ok(sender_pid) => Ok(Some(sender_pid)),
            Err(SysError::WouldBlock) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn recv(&self) -> Result<usize, Error> {
        loop {
            if let Some(sender_pid) = self.try_take()? {
                return Ok(sender_pid);
            }
            sys_wait(self.handle, Signal::READABLE)?;
        }
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}
