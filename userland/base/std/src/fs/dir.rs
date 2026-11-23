use core::fmt::Display;
use core::slice;

use vespertine_abi::protocol::{
    AbiDirEntry,
    PacketFlags,
    PacketHeader,
    PacketType,
    VESPER_MAGIC,
};
use vespertine_abi::{
    AccessRights,
    DirectoryOp,
    FileStat,
    HandleID,
    Invocation,
};
use vrt::syscall::{
    sys_close,
    sys_create_dir,
    sys_invoke,
    sys_stat,
    sys_unlink,
};

use crate::fs::{
    Path,
    resolve,
    split_parent_name,
};
use crate::socket::Socket;
use crate::{
    Error,
    Read,
};

extern crate alloc;

use alloc::string::String;

pub struct Dir(HandleID);

#[repr(C)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
}

impl Display for DirEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.kind {
            EntryKind::File => write!(f, "{}", self.name),
            EntryKind::Directory => write!(f, "{}/", self.name),
            EntryKind::Object => write!(f, "*{}*", self.name),
        }
    }
}

impl PartialEq<&str> for DirEntry {
    fn eq(&self, other: &&str) -> bool { self.name == *other }
}

#[repr(C)]
pub enum EntryKind {
    File,
    Directory,
    Object,
}

impl Display for EntryKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::File => write!(f, "file"),
            Self::Directory => write!(f, "directory"),
            Self::Object => write!(f, "object"),
        }
    }
}

pub struct ReadDir {
    read_end: Socket,
    expects_entry: bool,
    finished: bool,
}

impl ReadDir {
    fn read_entry(&mut self) -> Result<Option<DirEntry>, Error> {
        let mut header = PacketHeader::default();
        let header_bytes = unsafe {
            slice::from_raw_parts_mut((&mut header as *mut PacketHeader).cast::<u8>(), size_of::<PacketHeader>())
        };

        if !self.read_end.read_exact_or_eof(header_bytes)
            .map_err(|error| error.context("reading directory entry header"))?
        {
            if self.expects_entry {
                return Err(Error::end_of_stream("directory listing ended before the next entry".into()));
            }
            return Ok(None);
        }

        if header.magic != VESPER_MAGIC
            || header.version != 1
            || header.packet_type != PacketType::DirEntry as u32
            || header.payload_len as usize != size_of::<AbiDirEntry>()
        {
            return Err(Error::invalid_encoding("invalid directory entry header".into()));
        }

        let mut entry = AbiDirEntry { entry_type: 0, name_len: 0, name: [0; 254] };
        let entry_bytes = unsafe {
            slice::from_raw_parts_mut((&mut entry as *mut AbiDirEntry).cast::<u8>(), size_of::<AbiDirEntry>())
        };
        self.read_end.read_exact(entry_bytes)
            .map_err(|error| error.context("reading directory entry payload"))?;

        let name_bytes = entry.name.get(..entry.name_len as usize)
            .ok_or_else(|| Error::invalid_encoding("invalid directory entry name length".into()))?;
        let name = core::str::from_utf8(name_bytes)
            .map_err(|_| Error::invalid_encoding("directory entry name is not UTF-8".into()))?
            .into();

        let kind = match entry.entry_type {
            1 => EntryKind::Directory,
            2 => EntryKind::File,
            _ => EntryKind::Object,
        };

        self.expects_entry = header.packet_flags.contains(PacketFlags::HAS_NEXT);
        self.finished = !self.expects_entry;
        Ok(Some(DirEntry { name, kind }))
    }
}

impl Iterator for ReadDir {
    type Item = Result<DirEntry, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        let result = self.read_entry();
        if !matches!(&result, Ok(Some(_))) {
            self.finished = true;
        }
        result.transpose()
    }
}

impl Dir {
    pub fn open(path: &Path<'_>) -> Result<Self, Error> { Self::open_with_rights(path, AccessRights::LIST | AccessRights::TRAVERSE) }

    pub fn open_with_rights(path: &Path<'_>, rights: AccessRights) -> Result<Self, Error> { 
        resolve(path, rights)
            .map(Dir)
            .map_err(|error| error.context(format_args!("cannot open directory '{}'", path.as_str())))
    }

    pub fn handle(&self) -> HandleID { self.0 }

    pub fn from_handle(handle: HandleID) -> Self { Dir(handle) }

    pub fn list(&self) -> Result<ReadDir, Error> {
        let (read_end, write_end) = Socket::new_pair()?;

        let op = DirectoryOp::List { offset: 0, sink: write_end.handle() };

        sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;

        write_end.close();

        Ok(ReadDir { read_end, expects_entry: false, finished: false })
    }

    pub fn subdir(&self, name: &str) -> Result<Dir, Error> {
        let op = DirectoryOp::Lookup { name: name.as_ptr() as usize, name_len: name.len() };
        let handle = sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;
        Ok(Dir::from_handle(HandleID(handle)))
    }

    pub fn lookup(&self, name: &str) -> Result<HandleID, Error> {
        let op = DirectoryOp::Lookup { name: name.as_ptr() as usize, name_len: name.len() };
        let handle = sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;
        Ok(HandleID(handle))
    }

    pub fn create_dir(path: &Path<'_>) -> Result<Self, Error> {
        let context = |error: Error| {
            error.context(format_args!("cannot create directory '{}'", path.as_str()))
        };

        let (parent_path, dir_name) = split_parent_name(path)
            .map_err(|error| context(error.into()))?;
        let parent = resolve(&parent_path.as_path(), AccessRights::CREATE)
            .map_err(context)?;

        let res = sys_create_dir(parent, dir_name).map(Dir::from_handle);
        let _ = sys_close(parent);

        res.map_err(|error| context(error.into()))
    }

    pub fn remove(path: &Path<'_>) -> Result<(), Error> {
        let context = |error: Error| {
            error.context(format_args!("cannot delete '{}'", path.as_str()))
        };

        let (parent_path, name) = split_parent_name(path)
            .map_err(|error| context(error.into()))?;
        let parent = resolve(&parent_path.as_path(), AccessRights::REMOVE)
            .map_err(context)?;

        let res = sys_unlink(parent, name);
        let _ = sys_close(parent);

        res.map_err(|error| context(error.into()))
    }

    pub fn stat(&self) -> Result<FileStat, Error> {
        let mut stat = FileStat::zeroed();
        sys_stat(self.handle(), &mut stat)?;
        Ok(stat)
    }
}

impl Drop for Dir {
    fn drop(&mut self) { let _ = sys_close(self.0); }
}
