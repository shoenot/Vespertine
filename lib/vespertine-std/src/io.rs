use core::slice;

use crate::{Error, ErrorKind};
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use vespertine_abi::{HandleID, protocol::{PacketFlags, PacketHeader, PacketType, VESPER_MAGIC}};
use vespertine_rt::syscall::{sys_read, sys_write};

pub trait Read {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error>;

    fn read_to_end(&self) -> Result<Vec<u8>, Error> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            match self.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(e),
            }
        }
        Ok(buf)
    }

    fn read_to_string(&self) -> Result<String, Error> {
        let bytes = self.read_to_end()?;
        String::from_utf8(bytes).map_err(|_| Error {
            kind: ErrorKind::InvalidArgument,
            message: "Stream contains invalid UTF-8".into(),
        })
    }

    fn read_exact(&self, mut buf: &mut [u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => {
                    return Err(Error {
                        kind: ErrorKind::OutOfMemory,
                        message: "Unexpected end of stream during read".into(),
                    });
                }
                Ok(n) => buf = &mut buf[n..],
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

pub trait Write {
    fn write(&self, buf: &[u8]) -> Result<usize, Error>;

    fn write_all(&self, buf: &[u8]) -> Result<(), Error> {
        let mut total = 0;
        while total < buf.len() {
            match self.write(&buf[total..]) {
                Ok(0) => {
                    return Err(Error {
                        kind: ErrorKind::OutOfMemory,
                        message: "Write failed".into(),
                    });
                }
                Ok(n) => total += n,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn write_string(&self, s: String) -> Result<(), Error> {
        let bytes = s.as_bytes();
        self.write_all(bytes)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HandleReader {
    handle: HandleID,
}

impl HandleReader {
    pub fn new(handle: HandleID) -> Self {
        Self { handle }
    }
}

impl Read for HandleReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        sys_read(self.handle, buf.as_mut_ptr(), buf.len(), usize::MAX).map_err(Error::from)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HandleWriter {
    handle: HandleID,
}

impl HandleWriter {
    pub fn new(handle: HandleID) -> Self {
        Self { handle }
    }
}

impl Write for HandleWriter {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        sys_write(self.handle, buf.as_ptr(), buf.len(), usize::MAX).map_err(Error::from)
    }
}
