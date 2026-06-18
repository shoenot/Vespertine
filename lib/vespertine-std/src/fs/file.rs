pub use crate::path::*;
use crate::{Error, fs::resolve};
use crate::{
    ErrorKind,
    io::{Read, Write},
};
use core::ops::Drop;
use vespertine_abi::{AccessRights, FileOp, FileStat, HandleID, Invocation};
use vespertine_rt::syscall::{
    sys_close, sys_create_file, sys_invoke, sys_read, sys_stat, sys_write,
};

extern crate alloc;

pub enum SeekFrom {
    Start(usize),
    Current(i64),
    End(i64),
}

pub struct File {
    handle: HandleID,
}

impl File {
    pub fn open(path: &Path<'_>) -> Result<Self, Error> {
        Self::open_with_rights(path, AccessRights::READ)
    }

    pub fn open_with_rights(path: &Path<'_>, rights: AccessRights) -> Result<Self, Error> {
        resolve(path, rights).map(File::from_handle)
    }

    pub fn handle(&self) -> HandleID {
        self.handle
    }

    pub fn from_handle(handle: HandleID) -> Self {
        Self { handle }
    }

    pub fn stat(&self) -> Result<FileStat, Error> {
        let mut stat = FileStat::zeroed();
        sys_stat(self.handle, &mut stat)?;
        Ok(stat)
    }

    pub fn seek(&self, from: SeekFrom) -> Result<usize, Error> {
        let (offset, whence) = match from {
            SeekFrom::Start(offset) => (
                i64::try_from(offset).map_err(|_| Error {
                    kind: ErrorKind::InvalidArgument,
                    message: "".into(),
                })?,
                0,
            ),
            SeekFrom::Current(offset) => (offset, 1),
            SeekFrom::End(offset) => (offset, 2),
        };

        sys_invoke(
            self.handle,
            &Invocation::File(FileOp::Seek { offset, whence }),
        )
        .map_err(Error::from)
    }

    pub fn create(path: &Path<'_>) -> Result<Self, Error> {
        let (parent_path, dir_name) = split_parent_name(path).map_err(Error::from)?;
        let parent = resolve(&parent_path.as_path(), AccessRights::CREATE)?;
        let res = sys_create_file(parent, dir_name).map(File::from_handle);
        let _ = sys_close(parent);
        res.map_err(Error::from)
    }

    pub fn read_at(&self, buf: &mut [u8], offset: usize) -> Result<usize, Error> {
        sys_read(self.handle, buf.as_mut_ptr(), buf.len(), offset).map_err(Error::from)
    }

    pub fn write_at(&self, buf: &[u8], offset: usize) -> Result<usize, Error> {
        sys_write(self.handle, buf.as_ptr(), buf.len(), offset).map_err(Error::from)
    }
}

impl Read for File {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        sys_read(self.handle, buf.as_mut_ptr(), buf.len(), usize::MAX).map_err(Error::from)
    }
}

impl Write for File {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        sys_write(self.handle, buf.as_ptr(), buf.len(), usize::MAX).map_err(Error::from)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}
