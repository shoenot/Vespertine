use vespertine_abi::{HandleID, Signal, tag::TAG_SYS_SOCKFAC};
use vespertine_rt::syscall::{sys_close, sys_create_socket, sys_read, sys_set_nb, sys_wait, sys_write};

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

    pub fn read_handle(&self) -> Result<HandleID, Error> {
        self.read_handle.ok_or(Error { kind: ErrorKind::InvalidHandle, message: "No read handle!" })
    }

    pub fn write_handle(&self) -> Result<HandleID, Error> {
        self.write_handle.ok_or(Error { kind: ErrorKind::InvalidHandle, message: "No write handle!" })
    }

    pub fn close_write(&mut self) {
        if let Some(h) = self.write_handle.take() {
            let _ = sys_close(h);
        }
    }

    pub fn setnb(&self, nb: bool) -> Result<(), Error> {
        if let Some(r) = self.read_handle {
            sys_set_nb(r, nb).map_err(Error::from)?;
        }
        if let Some(w) = self.write_handle {
            sys_set_nb(w, nb).map_err(Error::from)?;
        }
        Ok(())
    }

    pub fn wait(&self, signal: Signal) -> Result<(), Error> {
        if signal.contains(Signal::READABLE) || signal.contains(Signal::PEER_CLOSED) {
            let handle = self.read_handle().map_err(|_| Error {
                kind: ErrorKind::AccessDenied,
                message: "Socket is write-only, cannot wait for read",
            })?;
            sys_wait(handle, signal).map_err(Error::from)?;
        } else if signal.contains(Signal::WRITABLE) {
            let handle = self.write_handle().map_err(|_| Error {
                kind: ErrorKind::AccessDenied,
                message: "Socket is read-only, cannot wait for write",
            })?;
            sys_wait(handle, signal).map_err(Error::from)?;
        }
        Ok(())
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
        let handle = self.write_handle.ok_or(Error { 
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
