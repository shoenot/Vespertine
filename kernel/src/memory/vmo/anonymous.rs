use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::{PagedBackingStore, MappingPage};
use crate::memory::pool::{PoolReservation, PoolState};
use crate::memory::Frame4K;
use crate::memory::PAGE_SIZE_4K;
use crate::object::invoke::InvocationError;
use crate::sync::TicketLock;

#[derive(Debug)]
pub struct AnonymousVmo {
    size: usize,
    pages: TicketLock<BTreeMap<usize, Arc<Frame4K>>>,
    _reservation: Option<PoolReservation>,
}

impl AnonymousVmo {
    pub fn new(size: usize) -> Arc<Self> {
        Self::with_reservation(size, None)
    }

    pub fn new_in_pool(size: usize, pool: &Arc<PoolState>) -> Result<Arc<Self>, InvocationError> {
        let reservation = pool.reserve(size)?;
        Ok(Self::with_reservation(size, Some(reservation)))
    }

    fn with_reservation(size: usize, reservation: Option<PoolReservation>) -> Arc<Self> {
        Arc::new(Self { size, pages: TicketLock::new(BTreeMap::new()), _reservation: reservation })
    }

    pub(crate) fn get_page(&self, offset: usize) -> Result<Arc<Frame4K>, ()> {
        if offset >= self.size {
            return Err(());
        }

        let page_index = offset / PAGE_SIZE_4K;
        let mut pages = self.pages.lock();

        if let Some(frame) = pages.get(&page_index) {
            return Ok(Arc::clone(frame));
        }

        let frame = Arc::new(Frame4K::allocate_zeroed().ok_or(())?);
        pages.insert(page_index, Arc::clone(&frame));
        Ok(frame)
    }
}

impl PagedBackingStore for AnonymousVmo {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        self.get_page(offset).map(|frame| frame.phys())
    }

    fn clone_range(&self, offset: usize, len: usize, pool: &Arc<PoolState>) -> Result<Arc<dyn PagedBackingStore>, InvocationError> {
        let page = PAGE_SIZE_4K;
        if offset % page != 0 || offset.checked_add(len).is_none() {
            return Err(InvocationError::InvalidArgument);
        }

        let child = AnonymousVmo::new_in_pool(len, pool)?;
        let copy_len = self.size.saturating_sub(offset).min(len);
        let first_page = offset / page;
        let end_page = first_page + copy_len.div_ceil(page);

        let source_frames = {
            let pages = self.pages.lock();
            let range = pages.range(first_page..end_page);

            let mut frames = Vec::new();
            frames.try_reserve_exact(range.clone().count())
                .map_err(|_| InvocationError::OutOfMemory)?;
            frames.extend(range.map(|(&index, frame)| (index, Arc::clone(frame))));
            frames
        };

        for (index, source) in source_frames {
            let dst_offset = (index - first_page) * page;
            let count = (copy_len - dst_offset).min(page);
            let dest = child.get_page(dst_offset)
                .map_err(|_| InvocationError::OutOfMemory)?;

            unsafe {
                core::arch::asm!(
                    "cld",
                    "rep movsb",
                    inout("rsi") source.virt() => _,
                    inout("rdi") dest.virt() => _,
                    inout("rcx") count => _,
                    options(nostack),
                );
            }
        }

        Ok(child)
    }

    fn page_for_mapping(&self, offset: usize) -> Result<MappingPage, ()> {
        self.get_page(offset).map(MappingPage::Owned)
    }
}
