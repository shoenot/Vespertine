use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use super::pic8259;
use crate::HHDMOFFSET;

const SV_OFFSET: usize = 0xF0;
const EOI_OFFSET: usize = 0xB0;
const TIMER_LVT_OFFSET: usize = 0x320;
const LAPIC_ID_OFFSET: usize = 0x20;
const TPR_OFFSET: usize = 0x80;
const DIVIDE_CONFIG_OFFSET: usize = 0x3E;
const INIT_COUNT_OFFSET: usize = 0x380;
const CURRENT_COUNT_OFFSET: usize = 0x390;

const IA32_APIC_BASE: usize = 0x1B;

pub struct Local_APIC {
    pub base_addr: usize
}

unsafe impl Send for Local_APIC {}
unsafe impl Sync for Local_APIC {}

pub unsafe fn get_apic_base() -> usize {
    let (lower, upper): (u32, u32);
    unsafe {
        asm!(
            "rdmsr", 
            in("ecx") IA32_APIC_BASE,
            out("eax") lower,
            out("edx") upper,
            options(att_syntax)
        )
    }
    let base_phys = ((upper as u64) << 32) | (lower as u64);
    (base_phys & !0xFFF) as usize
}

impl Local_APIC {
    pub fn init() -> Self {
        unsafe {
            pic8259::disable();
            let base_addr = get_apic_base() + *HHDMOFFSET;
            let lapic = Local_APIC { base_addr };
            lapic.write_reg(SV_OFFSET, lapic.read_reg(SV_OFFSET)  | (1 << 8) | 0xFF);
            lapic.write_reg(TPR_OFFSET, 0);
            lapic
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
        unsafe { self.write_reg(EOI_OFFSET, 0); }
    }

    pub fn id(&self) -> u32 {
        unsafe { self.read_reg(LAPIC_ID_OFFSET) }
    }

    pub fn timer_setup(&self, vector: u8, init_count: u32) {
        unsafe {
            self.write_reg(DIVIDE_CONFIG_OFFSET, 0x03);
            let mode_periodic = 0x20000;
            self.write_reg(TIMER_LVT_OFFSET, mode_periodic | vector as u32);
            self.write_reg(INIT_COUNT_OFFSET, init_count);
        }
    }

    pub fn stop_timer(&self) {
        unsafe { self.write_reg(INIT_COUNT_OFFSET, 0) };
    }
}
