use core::{
    arch::asm,
    ptr::{
        read_volatile,
        write_volatile,
    },
};

use lazy_static::lazy_static;

use super::pic8259;
use crate::memory::HHDMOFFSET;

const SV_OFFSET: usize = 0xF0;
const EOI_OFFSET: usize = 0xB0;
const TIMER_LVT_OFFSET: usize = 0x320;
const LAPIC_ID_OFFSET: usize = 0x20;
const TPR_OFFSET: usize = 0x80;
const DIVIDE_CONFIG_OFFSET: usize = 0x3E0;
const INIT_COUNT_OFFSET: usize = 0x380;
const CURRENT_COUNT_OFFSET: usize = 0x390;

const IA32_APIC_BASE: usize = 0x1B;

pub struct LocalAPIC {
    pub base_addr: usize,
}

unsafe impl Send for LocalAPIC {}
unsafe impl Sync for LocalAPIC {}

lazy_static! {
    static ref LAPIC_BASE_ADDR: usize = get_apic_base();
}

#[derive(Clone, Copy)]
pub enum TimerMode {
    OneShot = 0x00000,
    Periodic = 0x20000,
    TscDeadline = 0x40000,
}

pub fn get_apic_base() -> usize {
    let (lower, upper): (u32, u32);
    unsafe {
        asm!("rdmsr", 
            in("ecx") IA32_APIC_BASE,
            out("eax") lower,
            out("edx") upper)
    }
    let base_phys = ((upper as u64) << 32) | (lower as u64);
    (base_phys & !0xFFF) as usize
}

pub fn send_apic_eoi() {
    unsafe {
        let eoi_ptr = ((*LAPIC_BASE_ADDR + *HHDMOFFSET) + EOI_OFFSET) as *mut u32;
        write_volatile(eoi_ptr, 0);
    }
}

impl LocalAPIC {
    pub fn init(&mut self) {
        unsafe {
            pic8259::disable();
            self.base_addr = get_apic_base() + *HHDMOFFSET;
            self.write_reg(SV_OFFSET, self.read_reg(SV_OFFSET) | (1 << 8) | 0xFF);
            self.write_reg(TPR_OFFSET, 0);
        }
    }

    unsafe fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u32;
            write_volatile(ptr, value);
        }
    }

    unsafe fn read_reg(&self, offset: usize) -> u32 {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u32;
            read_volatile(ptr)
        }
    }

    pub fn eoi(&self) {
        unsafe {
            self.write_reg(EOI_OFFSET, 0);
        }
    }

    pub fn id(&self) -> u32 { unsafe { self.read_reg(LAPIC_ID_OFFSET) } }

    pub fn timer_setup(&self, vector: u8, init_count: u32, mode: TimerMode) {
        unsafe {
            self.write_reg(DIVIDE_CONFIG_OFFSET, 0x03);
            self.write_reg(TIMER_LVT_OFFSET, mode as u32 | vector as u32);

            if matches!(mode, TimerMode::TscDeadline) {
                self.write_reg(INIT_COUNT_OFFSET, 0);
            } else {
                self.write_reg(INIT_COUNT_OFFSET, init_count);
            }
        }
    }

    pub fn stop_timer(&self) { unsafe { self.write_reg(INIT_COUNT_OFFSET, 0) }; }

    pub fn current_count(&self) -> usize { unsafe { self.read_reg(CURRENT_COUNT_OFFSET) as usize } }

    pub fn arm_oneshot(&self, ticks: u32) {
        unsafe {
            self.write_reg(INIT_COUNT_OFFSET, ticks);
        }
    }
}
