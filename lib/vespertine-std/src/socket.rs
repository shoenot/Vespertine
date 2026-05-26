use vespertine_abi::{HandleID, tag::TAG_SYS_SOCKFAC};
use vespertine_rt::syscall::{sys_close, sys_create_socket, sys_read, sys_write};

use crate::{Error, ErrorKind, env, io::{Read, Write}};

pub struct Socket {
    read_handle: Option<HandleID>,
    write_handle: Option<HandleID>,
}

impl Socket {
    pub fn new() -> Result<Self, Error> {
        let sf = env::find_tag(TAG_SYS_SOCKFAC).expect("Socket Factory not found").id;
        let (r, w) = sys_create_socket(sf).map_err(Error::from)?;
        Ok(Socket { read_handle: Some(r), write_handle: Some(w) })
    }

    pub fn from_read_handle(handle: HandleID) -> Self {
        Socket { read_handle: Some(handle), write_handle: None }
    }

    pub fn from_write_handle(handle: HandleID) -> Self {
        Socket { read_handle: None, write_handle: Some(handle) }
    }

    pub fn close_write(&mut self) {
        if let Some(h) = self.write_handle.take() {
            let _ = sys_close(h);
        }
    }
}

impl Read for Socket {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let handle = self.read_handle.ok_or(Error { 
            kind: ErrorKind::AccessDenied, 
            message: "Socket is write-only" 
        })?;

        sys_read(handle, buf.as_mut_ptr(), buf.len(), 0)
            .map_err(Error::from)
    }
}

impl Write for Socket {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        let handle = self.read_handle.ok_or(Error { 
            kind: ErrorKind::AccessDenied, 
            message: "Socket is read-only" 
        })?;

        sys_write(handle, buf.as_ptr(), buf.len(), 0)
            .map_err(Error::from)
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        if let Some(h) = self.read_handle { let _ = sys_close(h); }
        if let Some(h) = self.write_handle { let _ = sys_close(h); }
    }
}
