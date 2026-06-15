use crate::Error;
pub use crate::path::*;
use crate::{
    ErrorKind,
    fs::parse_parent_and_name,
    io::{Read, Write},
};
use core::cell::Cell;
use core::ops::Drop;
use vespertine_abi::{AccessRights, FileOp, HandleID, Invocation};
use vespertine_rt::syscall::{sys_close, sys_create_file, sys_invoke, sys_read, sys_write};

extern crate alloc;

pub struct File {
    pub handle: HandleID,
}

impl File {
    pub fn open(path: &str) -> Result<Self, Error> {
        Self::open_with_rights(path, AccessRights::READ)
    }

    pub fn open_with_rights(path: &str, rights: AccessRights) -> Result<Self, Error> {
        walk_path(path, rights).map(File::from).map_err(Error::from)
    }

    pub fn from(handle: HandleID) -> Self {
        Self {
            handle,
            cursor: Cell::new(0),
        }
    }

    pub fn stat(&self) -> Result<usize, Error> {
        let op = FileOp::Stat;
        sys_invoke(self.handle, &Invocation::File(op)).map_err(Error::from)
    }

    pub fn seek(&self, pos: usize) {
        self.cursor.set(pos);
    }

    pub fn create(path: &str) -> Result<Self, Error> {
        let (parent_path, file_name) = parse_parent_and_name(path);
        let parent_handle = walk_path(parent_path, AccessRights::CREATE)?;
        let handle = sys_create_file(parent_handle, file_name).map_err(Error::from)?;

        let _ = sys_close(parent_handle);

        Ok(File::from(handle))
    }
}

impl Read for File {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let offset = self.cursor.get();
        match sys_read(self.handle, buf.as_mut_ptr(), buf.len(), offset) {
            Ok(n) => {
                self.cursor.set(offset + n);
                Ok(n)
            }
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
            }
            Err(e) => Err(Error::from(e)),
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}
