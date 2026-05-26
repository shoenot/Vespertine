use vespertine_abi::{DirectoryOp, FileOp, HandleID, Invocation};
use vespertine_rt::syscall::{SysError, sys_close, sys_invoke, sys_read, sys_write};
use core::cell::Cell;
use core::ops::Drop;
use crate::io::{Read, Write};
pub use crate::path::*;
use crate::{Error, ErrorKind};

extern crate alloc;
use alloc::vec::Vec;
use alloc::string::String;

pub struct File { 
    pub handle: HandleID,
    cursor: Cell<usize>,
}

impl File {
    pub fn open(path: &str) -> Result<Self, Error> {
        walk_path(path, HandleID(0))
            .map(|handle| File { handle, cursor: Cell::new(0) })
            .map_err(Error::from)
    }

    pub fn from(handle: HandleID) -> Self {
        Self { handle, cursor: Cell::new(0) }
    }

    pub fn stat(&self) -> Result<usize, Error>{
        let op = FileOp::Stat;
        sys_invoke(self.handle, &Invocation::File(op)).map_err(Error::from)
    }

    pub fn seek(&self, pos: usize) {
        self.cursor.set(pos);
    }
}

impl Read for File {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let offset = self.cursor.get();
        match sys_read(self.handle, buf.as_mut_ptr(), buf.len(), offset) {
            Ok(n) => {
                self.cursor.set(offset + n);
                Ok(n)
            },
            Err(e) => Err(Error::from(e)),
        }
    }
}

impl Write for File {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        let offset = self.cursor.get();
        match sys_write(self.handle, buf.as_ptr(), buf.len(), offset) {
            Ok(n) => {
                self.cursor.set(offset + n);
                Ok(n)
            },
            Err(e) => Err(Error::from(e)),
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}



