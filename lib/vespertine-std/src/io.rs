use crate::{Error, ErrorKind};
extern crate alloc;
use alloc::vec::Vec;
use alloc::string::String;

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
            message: "Stream contains invalid UTF-8",
        })      
    }
}

pub trait Write {
    fn write(&self, buf: &[u8]) -> Result<usize, Error>;

    fn write_all(&self, buf: &[u8]) -> Result<(), Error> {
        let mut total = 0;
        while total < buf.len() {
            match self.write(&buf[total..]) {
                Ok(0) => return Err(Error { kind: ErrorKind::OutOfMemory, message: "Write failed" }),
                Ok(n) => total += n,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
