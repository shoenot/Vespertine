use core::fmt::Display;
use core::ptr::copy_nonoverlapping;

use vespertine_abi::protocol::{AbiDirEntry, DirEntryType, PacketFlags, PacketType, VESPER_MAGIC};
use vespertine_abi::{DirectoryOp, FileOp, HandleID, Invocation, protocol::PacketHeader, tag::TAG_SYS_SOCKFAC};
use vespertine_rt::syscall::{sys_close, sys_create_socket, sys_invoke, sys_read};

use crate::{Error, ErrorKind, env::find_tag, fs::walk_path};

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

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
    read_handle: HandleID,
    finished: bool,
    buffer: [u8; 4096],
    cursor: usize,
    limit: usize,
}

pub static FULL_ENTRY: usize = size_of::<PacketHeader>() + size_of::<AbiDirEntry>();

impl Iterator for ReadDir {
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished { return None };

        let remaining = self.limit - self.cursor;
        // ensure buffer holds at least one complete entry
        if remaining < FULL_ENTRY {
            // shift unparsed leftovers to front
            if remaining > 0 {
                self.buffer.copy_within(self.cursor..self.limit, 0);
            }
            self.cursor = 0;
            self.limit = remaining;

            let to_read = 4096 - self.limit;
            let read_ptr = unsafe { self.buffer.as_mut_ptr().add(self.limit) };

            match sys_read(self.read_handle, read_ptr, to_read, 0) {
                Ok(n) if n > 0 => {
                    self.limit += n;
                },
                _ => {
                    // eof or read error
                    if self.limit - self.cursor < 280 {
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
                header_len
            );
        }
        self.cursor += header_len;

        // verify magic number 
        if header.magic != VESPER_MAGIC {
            self.finished = true;
            return None;
        }

        // read payload
        let mut entry = AbiDirEntry { entry_type: 0, name_len: 0, name: [0u8; 254], };
        let entry_len = size_of::<AbiDirEntry>();
        unsafe {
            copy_nonoverlapping(
                self.buffer.as_ptr().add(self.cursor), 
                &mut entry as *mut _ as *mut u8, 
                entry_len
            );
        }
        self.cursor += entry_len;

        if !header.packet_flags.contains(PacketFlags::HAS_NEXT) {
            self.finished = true;
        }

        let name_bytes = &entry.name[..entry.name_len as usize];
        let name = str::from_utf8(name_bytes)
            .unwrap_or("Invalid UTF-8")
            .into();

        let kind = match entry.entry_type {
            1 => EntryKind::Directory,
            2 => EntryKind::File,
            _ => EntryKind::Object
        };

        Some(DirEntry { name, kind })
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        let _ = sys_close(self.read_handle);
    }
}

impl Dir {
    pub fn open(path: &str) -> Result<Self, Error> {
        walk_path(path, HandleID(0))
            .map(Dir)
            .map_err(Error::from)
    }

    pub fn from(handle: HandleID) -> Self {
        Dir(handle)
    }

    pub fn list(&self) -> Result<ReadDir, Error> {
        let sf = find_tag(TAG_SYS_SOCKFAC)
            .ok_or(Error { kind: ErrorKind::NotFound, message: "Socket factory not found" })?;
        let (read_end, write_end) = sys_create_socket(sf.id)?;

        let op = DirectoryOp::List { offset: 0, sink: write_end };
        sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;
        let _ = sys_close(write_end);

        Ok(ReadDir { 
            read_handle: read_end, 
            finished: false,
            buffer: [0u8; 4096],
            cursor: 0,
            limit: 0,
        })
    }

    pub fn subdir(&self, name: &'static str) -> Result<Dir, Error> {
        let op = DirectoryOp::Lookup { name: name.as_ptr(), name_len: name.len() };
        let handle = sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;
        Ok(Dir::from(HandleID(handle)))
    }

    pub fn lookup(&self, name: &'static str) -> Result<HandleID, Error> {
        let op = DirectoryOp::Lookup { name: name.as_ptr(), name_len: name.len() };
        let handle = sys_invoke(self.0, &Invocation::Directory(op)).map_err(Error::from)?;
        Ok(HandleID(handle))
    }
}
