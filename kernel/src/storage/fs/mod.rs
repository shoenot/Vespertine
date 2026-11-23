use alloc::boxed::Box;
use core::fmt::Debug;

use async_trait::async_trait;
pub use ext2::init::mount_ext2_rootfs;

use crate::object::invoke::InvocationError;

mod ext2;
#[cfg(feature = "diagnostics")]
pub(crate) use ext2::obj::Ext2Directory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsNodeType {
    File,
    Directory,
    Symlink,
    Special,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsReadError {
    InvalidArgument,
    OutOfMemory,
    Io,
}

impl From<VfsReadError> for InvocationError {
    fn from(error: VfsReadError) -> Self {
        match error {
            VfsReadError::InvalidArgument => Self::InvalidArgument,
            VfsReadError::OutOfMemory => Self::OutOfMemory,
            VfsReadError::Io => Self::Io,
        }
    }
}

#[async_trait]
pub trait VfsNode: Send + Sync + Debug {
    /// Read from a page-aligned file offset into a 4 KiB physical buffer.
    /// The buffer may be filled up to the last disk block covered by the read.
    async fn read_at_phys(&self, offset: usize, dest_phys: usize, len: usize) -> Result<usize, VfsReadError>;

    fn size(&self) -> usize;

    fn node_type(&self) -> VfsNodeType;
}
