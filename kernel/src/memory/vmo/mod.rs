use alloc::sync::Arc;
use core::fmt::Debug;

use crate::memory::pool::PoolState;
use crate::memory::Frame4K;
use crate::object::invoke::InvocationError;
use hal::mmu::{PageFlags, PhysAddr};

mod anonymous;
pub use anonymous::AnonymousVmo;
mod file;
pub use file::FileVmo;
mod device;
pub use device::DeviceVmo;

pub enum MappingPage {
    Owned(Arc<Frame4K>),
    Device { phys: PhysAddr, cache: PageFlags },
}

pub trait PagedBackingStore: Send + Sync + Debug {
    fn request_page(&self, offset: usize) -> Result<usize, ()>;

    fn clone_range(&self, _offset: usize, _len: usize, _pool: &Arc<PoolState>) -> Result<Arc<dyn PagedBackingStore>, InvocationError> {
        Err(InvocationError::UnsupportedOperation)
    }

    fn page_for_mapping(&self, offset: usize) -> Result<MappingPage, ()>;
    fn device_mapping_len(&self) -> Option<usize> { None }
}
