use core::ptr::{
    read_volatile,
    write_volatile,
};

use crate::x86_64::platform;

const HPET_GEN_CAP_OFFSET: usize = 0x0;
const HPET_GEN_CONF_OFFSET: usize = 0x10;
const HPET_MAIN_COUNTER_OFFSET: usize = 0xF0;

#[derive(Copy, Clone, Debug)]
pub(crate) struct Hpet {
    pub(crate) base_addr: usize,
    pub(crate) frequency: usize,
    pub(crate) enabled: bool,
}

impl Hpet {
    pub(crate) fn new(phys_addr: usize) -> Self {
        let base_addr = platform::map_mmio(phys_addr as u64, 4096).expect("failed to map HPET MMIO");

        let hpet = Self {
            base_addr,
            frequency: 0,
            enabled: false,
        };

        let capabilities = hpet.read_reg(HPET_GEN_CAP_OFFSET);
        let tick_len = capabilities >> 32;
        let frequency = 1_000_000_000_000_000 / tick_len;

        Self {
            base_addr,
            frequency: frequency as usize,
            enabled: false,
        }
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

    pub(crate) fn read_counter(&self) -> usize {
        self.read_reg(HPET_MAIN_COUNTER_OFFSET) as usize
    }

    fn write_reg(&self, offset: usize, value: u64) {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u64;
            write_volatile(ptr, value);
        }
    }

    fn read_reg(&self, offset: usize) -> u64 {
        unsafe {
            let ptr = (self.base_addr + offset) as *const u64;
            read_volatile(ptr)
        }
    }
}

pub(crate) fn read_hpet_counter(base_addr: usize) -> usize {
    unsafe {
        read_volatile((base_addr + HPET_MAIN_COUNTER_OFFSET) as *const usize)
    }
}
