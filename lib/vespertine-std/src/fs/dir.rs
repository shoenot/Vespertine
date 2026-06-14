use core::fmt::Display;
use core::ptr::copy_nonoverlapping;

use vespertine_abi::AccessRights;
use vespertine_abi::protocol::{AbiDirEntry, PacketFlags, VESPER_MAGIC};
use vespertine_abi::{
    DirectoryOp, HandleID, Invocation, protocol::PacketHeader,
};
use vespertine_rt::syscall::{
    sys_close, sys_create_dir, sys_create_socket, sys_invoke, sys_read, sys_unlink,
};

use crate::Read;
use crate::fs::parse_parent_and_name;
use crate::socket::Socket;
use crate::{Error, ErrorKind, fs::walk_path};

extern crate alloc;

use alloc::string::String;

pub struct Dir(pub HandleID);

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
    fn eq(&self, other: &&str) -> bool {
        self.name == *other
    }
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
    finished: bool,
    buffer: [u8; 4096],
    cursor: usize,
    limit: usize,
}

pub static FULL_ENTRY: usize = size_of::<PacketHeader>() + size_of::<AbiDirEntry>();

impl Iterator for ReadDir {
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        };

        let remaining = self.limit - self.cursor;
        // ensure buffer holds at least one complete entry
        if remaining < FULL_ENTRY {
            // shift unparsed leftovers to front
            if remaining > 0 {
                self.buffer.copy_within(self.cursor..self.limit, 0);
            }
            self.cursor = 0;
            self.limit = remaining;

            match self.read_end.read(&mut self.buffer[self.limit..]) {
                Ok(n) if n > 0 => {
                    self.limit += n;
                }
                _ => {
                    // eof or read error
                    if self.limit - self.cursor < FULL_ENTRY {
                        self.finished = true;
                        return None;
                    }
                }
            }
        }

        let mut header = PacketHeader {
            magic: 0,
            version: 0,
            packet_flags: PacketFlags::new(),
            packet_type: 0,
            payload_len: 0,
            reserved: 0,
        };

        // read header
        let header_len = size_of::<PacketHeader>();

        unsafe {
            copy_nonoverlapping(
                self.buffer.as_ptr().add(self.cursor),
                &mut header as *mut _ as *mut u8,
                header_len,
            );
        }
        self.cursor += header_len;

        // verify magic number
        if header.magic != VESPER_MAGIC {
            self.finished = true;
            return None;
        }

        // read payload
        let mut entry = AbiDirEntry {
            entry_type: 0,
            name_len: 0,
            name: [0u8; 254],
        };
        let entry_len = size_of::<AbiDirEntry>();
        unsafe {
            copy_nonoverlapping(
                self.buffer.as_ptr().add(self.cursor),
                &mut entry as *mut _ as *mut u8,
                entry_len,
            );
        }
        self.cursor += entry_len;

        if !header.packet_flags.contains(PacketFlags::HAS_NEXT) {
            self.finished = true;
        }

        let name_bytes = &entry.name[..entry.name_len as usize];
        let name = str::from_utf8(name_bytes).unwrap_or("Invalid UTF-8").into();

        let kind = match entry.entry_type {
            1 => EntryKind::Directory,
            2 => EntryKind::File,
            _ => EntryKind::Object,
        };

        Some(DirEntry { name, kind })
    }
}

impl Dir {
    pub fn open(path: &str) -> Result<Self, Error> {
        Self::open_with_rights(path, AccessRights::LIST | AccessRights::TRAVERSE)
    }

    pub fn open_with_rights(path: &str, rights: AccessRights) -> Result<Self, Error> {
        walk_path(path, rights).map(Dir).map_err(Error::from)
    }

    pub fn from(handle: HandleID) -> Self {
        Dir(handle)
    }

    pub fn list(&self) -> Result<ReadDir, Error> {
        let (read_end, write_end) = Socket::new_pair()?;

        let op = DirectoryOp::List {
            offset: 0,
            sink: write_end.handle(),
        };

        sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;

        write_end.close();

        Ok(ReadDir {
            read_end,
            finished: false,
            buffer: [0u8; 4096],
            cursor: 0,
            limit: 0,
        })
    }

    pub fn subdir(&self, name: &str) -> Result<Dir, Error> {
        let op = DirectoryOp::Lookup {
            name: name.as_ptr() as usize,
            name_len: name.len(),
        };
        let handle = sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;
        Ok(Dir::from(HandleID(handle)))
    }

    pub fn lookup(&self, name: &str) -> Result<HandleID, Error> {
        let op = DirectoryOp::Lookup {
            name: name.as_ptr() as usize,
            name_len: name.len(),
        };
        let handle = sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;
        Ok(HandleID(handle))
    }

    pub fn create_dir(path: &str) -> Result<Self, Error> {
        let (parent_path, dir_name) = parse_parent_and_name(path);
        let parent_handle = walk_path(parent_path, AccessRights::CREATE)?;
        let handle = sys_create_dir(parent_handle, dir_name).map_err(Error::from)?;

        let _ = sys_close(parent_handle);

        Ok(Dir::from(handle))
    }

    pub fn remove(path: &str) -> Result<(), Error> {
        let (parent_path, name) = parse_parent_and_name(path);
        let parent_handle = walk_path(parent_path, AccessRights::REMOVE)?;
        sys_unlink(parent_handle, name).map_err(Error::from)?;
        let _ = sys_close(parent_handle);
        Ok(())
    }
}
