use vespertine_abi::{HandleID, tag::TAG_SYS_LOGGER};
use vespertine_rt::syscall::sys_write;

use crate::{Error, ErrorKind, Write, env::find_tag};

pub struct SystemLog(HandleID);

impl SystemLog {
    pub fn connect() -> Self {
        let handle = find_tag(TAG_SYS_LOGGER).expect("Could not find logger").id;
        Self(handle)
    }
}

impl Write for SystemLog {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        sys_write(self.0, buf.as_ptr(), buf.len(), 0).map_err(Error::from)
    }
}
