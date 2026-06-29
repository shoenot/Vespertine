use vespertine_abi::{
    AccessRights,
    HandleID,
};
use vespertine_rt::syscall::{
    sys_close,
    sys_read,
    sys_write,
};

use crate::fs::{
    Path,
    resolve,
};
use crate::{
    Error,
    Read,
    Write,
};

pub struct SystemLog(HandleID);

impl SystemLog {
    pub fn try_connect() -> Result<Self, Error> {
        let handle = resolve(&Path::new("/System/Services/Log"), AccessRights::WRITE)?;
        Ok(Self(handle))
    }

    pub fn connect() -> Self { Self::try_connect().expect("Could not find logger") }

    pub fn from_handle(handle: HandleID) -> Self { Self(handle) }

    pub fn handle(&self) -> HandleID { self.0 }

    pub fn info(&self, message: &str) -> Result<(), Error> { self.write_all(message.as_bytes()) }

    pub fn warn(&self, message: &str) -> Result<(), Error> { self.write_all(message.as_bytes()) }

    pub fn error(&self, message: &str) -> Result<(), Error> { self.write_all(message.as_bytes()) }
}

pub struct LogReader(HandleID);

impl LogReader {
    pub fn from_handle(handle: HandleID) -> Self { Self(handle) }
}

impl Read for LogReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> { sys_read(self.0, buf.as_mut_ptr(), buf.len(), 0).map_err(Error::from) }
}

impl Write for SystemLog {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> { sys_write(self.0, buf.as_ptr(), buf.len(), 0).map_err(Error::from) }
}

impl Drop for SystemLog {
    fn drop(&mut self) { let _ = sys_close(self.0); }
}
