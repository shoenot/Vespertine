use core::ptr::{
    read_volatile,
    write_volatile,
};
use core::sync::atomic::Ordering;

use crate::core::time::{
    ClockSource,
    HPET_BASE_ADDR,
};
use crate::memory::HHDMOFFSET;

const HPET_GEN_CAP_OFFSET: usize = 0x0;
const HPET_GEN_CONF_OFFSET: usize = 0x10;
const HPET_MAIN_COUNTER_OFFSET: usize = 0xF0;

#[derive(Copy, Clone, Debug)]
pub(crate) struct HPET {
    pub base_addr: usize,
    pub frequency: usize,
    pub enabled: bool,
}

impl HPET {
    pub(crate) fn init(&mut self, base_addr: usize) {
        self.base_addr = base_addr + *HHDMOFFSET;
        let capabilites = self.read_reg(HPET_GEN_CAP_OFFSET);
        let tick_len = capabilites >> 32;
        let fq = 1_000_000_000_000_000 / tick_len;
        self.frequency = fq as usize;
        self.enabled = false;
    }

    pub(crate) fn enable(&mut self) {
        let existing = self.read_reg(HPET_GEN_CONF_OFFSET);
        self.write_reg(HPET_GEN_CONF_OFFSET, existing | 1);
        self.enabled = true;
    }

    pub(crate) fn disable(&mut self) {
        let existing = self.read_reg(HPET_GEN_CONF_OFFSET);
        self.write_reg(HPET_GEN_CONF_OFFSET, existing & !1);
        self.enabled = false;
    }

    fn write_reg(&self, offset: usize, value: u64) {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u64;
            write_volatile(ptr, value);
        }
    }

    fn read_reg(&self, offset: usize) -> u64 {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u64;
            read_volatile(ptr)
        }
    }
}

impl ClockSource for HPET {
    fn name(&self) -> &'static str { "HPET" }

    fn read_counter(&self) -> usize { self.read_reg(HPET_MAIN_COUNTER_OFFSET) as usize }

    fn frequency(&self) -> usize { self.frequency }
}

pub(crate) fn read_hpet_direct() -> usize {
    let addr = HPET_BASE_ADDR.load(Ordering::Relaxed) as usize + HPET_MAIN_COUNTER_OFFSET;
    unsafe { read_volatile(addr as *const usize) }
}
