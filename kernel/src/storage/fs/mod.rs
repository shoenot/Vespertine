use core::fmt::Debug;
use alloc::boxed::Box;

use async_trait::async_trait;

pub use ext2::mount_ext2_rootfs;

mod ext2;

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

    async fn write_at_phys(&self, offset: usize, src_phys: usize, len: usize) -> Result<usize, ()>;

    fn size(&self) -> usize;

    fn resize(&self, new_size: usize) -> Result<(), ()>;

    fn node_type(&self) -> VfsNodeType;
}

