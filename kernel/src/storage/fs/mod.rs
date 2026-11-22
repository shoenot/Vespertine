use alloc::boxed::Box;
use core::fmt::Debug;

use async_trait::async_trait;
pub use ext2::init::mount_ext2_rootfs;

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

#[async_trait]
pub trait VfsNode: Send + Sync + Debug {
    async fn read_at_phys(&self, offset: usize, dest_phys: usize, len: usize) -> Result<usize, ()>;

    fn size(&self) -> usize;

    fn node_type(&self) -> VfsNodeType;
}
