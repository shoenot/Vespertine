use crate::memory::{
    PageSize,
    GLOBAL_PMM,
};

pub const MAG_CAPACITY: usize = 256;

pub struct Magazine {
    pages: [usize; MAG_CAPACITY], // memory addrs
    count: usize,
}

impl Magazine {
    pub const fn init() -> Self { Self { pages: [0; MAG_CAPACITY], count: 0 } }

    pub fn alloc(&mut self) -> Option<usize> {
        if self.count != 0 {
            self.count -= 1;
            return Some(core::mem::replace(&mut self.pages[self.count], 0));
        } 

        let mut pmm = GLOBAL_PMM.lock();
        let requested = pmm.alloc(PageSize::Size4K)?;

        for _ in 0..(MAG_CAPACITY / 2 - 1) {
            let Some(page) = pmm.alloc(PageSize::Size4K) else { break };
            self.pages[self.count] = page;
            self.count += 1;
        }

        Some(requested)
    }

    pub fn free(&mut self, addr: usize) {
        if self.count < MAG_CAPACITY {
            self.pages[self.count] = addr;
            self.count += 1;
        } else {
            let mut pmm = GLOBAL_PMM.lock();
            for _ in 0..(MAG_CAPACITY / 2) {
                self.count -= 1;
                pmm.free(self.pages[self.count], PageSize::Size4K);
            }
            self.pages[self.count] = addr;
            self.count += 1;
            drop(pmm);
        }
    }
}
