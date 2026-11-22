use alloc::sync::Arc;
use hal::mmu::{PageFlags, PageSize, PhysAddr, VirtAddr};

use crate::memory::{PAGE_SIZE_4K, PAGER, vmo::{MappingPage, PagedBackingStore}};



#[derive(Debug)]
pub struct DeviceVmo {
    base: PhysAddr,
    len: usize,
    mapped_len: usize,
    cache: PageFlags,
}

fn cache_for_4k(size: PageSize, flags: PageFlags) -> PageFlags {
    let mut cache = flags & (PageFlags::WRITE_THROUGH | PageFlags::NO_CACHE);
    let pat = if size == PageSize::Size4K {
        PageFlags::PAT_4K
    } else {
        PageFlags::PAT_LARGE
    };

    if flags.contains(pat) {
        cache = cache | PageFlags::PAT_4K;
    }

    cache
}

impl DeviceVmo {
    pub fn from_kernel_mapping(virt: usize, len: usize) -> Option<Arc<Self>> {
        if virt % PAGE_SIZE_4K != 0 || len == 0 { return None };

        let mapped_len = len.div_ceil(PAGE_SIZE_4K).checked_mul(PAGE_SIZE_4K)?;
        let pager = PAGER.lock();

        let (base, size, flags) = pager.translate(VirtAddr(virt))?;
        let cache = cache_for_4k(size, flags);
        base.0.checked_add(mapped_len)?;

        for offset in (0..mapped_len).step_by(PAGE_SIZE_4K) {
            let addr = VirtAddr(virt.checked_add(offset)?);
            let (phys, size, flags) = pager.translate(addr)?;
            if phys.0 != base.0 + offset || cache_for_4k(size, flags) != cache {
                return None;
            }
        }

        Some(Arc::new(Self { base, len, mapped_len, cache }))
    }
}

impl PagedBackingStore for DeviceVmo {
    fn request_page(&self, offset: usize) -> Result<usize, ()> {
        if offset >= self.len { return Err(()) };
        Ok(self.base.0 + (offset - offset % PAGE_SIZE_4K))
    }

    fn page_for_mapping(&self, offset: usize) -> Result<MappingPage, ()> {
        if offset % PAGE_SIZE_4K != 0 || offset >= self.mapped_len { return Err(()) };
        Ok(MappingPage::Device { phys: PhysAddr(self.base.0 + offset), cache: self.cache })
    }

    fn device_mapping_len(&self) -> Option<usize> {
        Some(self.mapped_len)
    }
}
