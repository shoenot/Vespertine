use core::{ptr, slice};

use crate::memory::PhysBuffer;

use super::structs::{DiskDirHeader, DiskInode};
use super::{Ext2Error, Ext2FileSystem};

const INDEXED_DIRECTORY: u32 = 0x0000_1000;

pub(super) struct DirectoryRecord<'a> {
    pub header: DiskDirHeader,
    pub name: &'a [u8],
    pub offset: usize,
    pub previous_offset: Option<usize>,
}

impl Ext2FileSystem {
    pub(super) fn directory_block_count(&self, inode: &DiskInode) -> Result<usize, Ext2Error> {
        let block_size = self.metadata_block_size()?;
        if inode.mode & 0xf000 != 0x4000 || inode.size % block_size != 0 {
            return Err(Ext2Error::InvalidMetadata);
        }
        Ok((inode.size / block_size) as usize)
    }

    pub(super) fn check_directory_writable(&self, inode: &DiskInode) -> Result<(), Ext2Error> {
        if inode.flags & INDEXED_DIRECTORY != 0 {
            return Err(Ext2Error::UnsupportedDirectoryLayout);
        }
        Ok(())
    }

    /// On a match, the caller's buffer still contains the matching block.
    pub(super) async fn walk_directory<T>(
        &self,
        inode: &DiskInode,
        buffer: &PhysBuffer,
        mut visit: impl FnMut(DirectoryRecord<'_>) -> Result<Option<T>, Ext2Error> + Send,
    ) -> Result<Option<(u32, T)>, Ext2Error> {
        const HEADER_SIZE: usize = size_of::<DiskDirHeader>();

        let count = self.directory_block_count(inode)?;
        let block_size = self.block_size as usize;
        for index in 0..count {
            let block = self.resolve_file_block(inode, index).await?;
            if block == 0 {
                return Err(Ext2Error::InvalidMetadata);
            }
            self.read_metadata_block(block, buffer).await?;

            let bytes = unsafe { slice::from_raw_parts(buffer.virt() as *const u8, block_size) };
            let mut offset = 0;
            let mut previous_offset = None;
            while offset < block_size {
                let record = &bytes[offset..];
                if record.len() < HEADER_SIZE {
                    return Err(Ext2Error::InvalidMetadata);
                }
                let header = unsafe { ptr::read(record.as_ptr().cast::<DiskDirHeader>()) };
                let length = header.record_length as usize;
                let name_end = HEADER_SIZE + header.name_length as usize;
                if length % 4 != 0 || length > record.len() || name_end > length
                    || (!self.directory_file_types && header.file_type != 0)
                {
                    return Err(Ext2Error::InvalidMetadata);
                }

                let name = &record[HEADER_SIZE..name_end];
                if let Some(found) = visit(DirectoryRecord { header, name, offset, previous_offset })? {
                    return Ok(Some((block, found)));
                }
                previous_offset = Some(offset);
                offset += length;
            }
        }
        Ok(None)
    }

    pub async fn lookup_in_dir(&self, inode: &DiskInode, name: &str) -> Result<Option<u32>, Ext2Error> {
        let buffer = PhysBuffer::new().ok_or(Ext2Error::OutOfMemory)?;
        let entry = self.walk_directory(inode, &buffer, |record| {
            let inode = record.header.inode;
            Ok((inode != 0 && record.name == name.as_bytes()).then_some(inode))
        }).await?;
        Ok(entry.map(|(_, inode)| inode))
    }
}
