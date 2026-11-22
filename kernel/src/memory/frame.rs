use hal::mmu::{DIRECT_MAP_OFFSET, PageSize};

use crate::memory::GLOBAL_PMM;



#[derive(Debug)]
pub struct Frame4K {
    phys: usize,
}

impl Frame4K {
    pub fn allocate() -> Option<Self> {
        let phys = GLOBAL_PMM.lock().alloc(PageSize::Size4K)?;
        Some(Self { phys })
    }

    pub fn allocate_zeroed() -> Option<Self> {
        let frame = Self::allocate()?;

        unsafe {
            core::ptr::write_bytes(frame.virt() as *mut u8, 0, PageSize::Size4K.bytes());
        }
        Some(frame)
    }

    pub fn phys(&self) -> usize {
        self.phys
    }

    pub fn virt(&self) -> usize {
        self.phys + *DIRECT_MAP_OFFSET
    }
}

impl Drop for Frame4K {
    fn drop(&mut self) {
        GLOBAL_PMM.lock().free(self.phys, PageSize::Size4K);
    }
}
