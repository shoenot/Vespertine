use core::arch::x86_64::__cpuid;
use core::ptr::{
    read_volatile,
    write_volatile,
};

use super::pic8259;
use crate::arch::get_core_data;
use crate::core::sync::KernelOnceCell;
use crate::memory::HHDMOFFSET;
use crate::util::bitwise::check_bit;
use crate::util::{
    read_from_msr,
    write_to_msr,
};

const SV_OFFSET: usize = 0xF0;
const EOI_OFFSET: usize = 0xB0;
const TIMER_LVT_OFFSET: usize = 0x320;
const LAPIC_ID_OFFSET: usize = 0x20;
const TPR_OFFSET: usize = 0x80;
const DIVIDE_CONFIG_OFFSET: usize = 0x3E0;
const INIT_COUNT_OFFSET: usize = 0x380;
const CURRENT_COUNT_OFFSET: usize = 0x390;

const IA32_APIC_BASE: usize = 0x1B;

const RESCHEDULE_IPI_VECTOR: u32 = 40;
const TLB_SHOOTDOWN_IPI_VECTOR: u32 = 41;

#[derive(Clone)]
pub(crate) struct XApicDriver {
    pub base_addr: usize,
}

#[derive(Clone)]
pub(crate) struct X2ApicDriver {
    pub base_addr: usize,
}

#[derive(Clone)]
pub enum ApicMode {
    XApic(XApicDriver),
    X2Apic(X2ApicDriver),
}

unsafe impl Send for XApicDriver {}
unsafe impl Sync for XApicDriver {}

unsafe impl Send for X2ApicDriver {}
unsafe impl Sync for X2ApicDriver {}

static LAPIC_BASE_ADDR: KernelOnceCell<usize> = KernelOnceCell::new();

#[derive(Clone, Copy)]
pub(crate) enum TimerMode {
    OneShot = 0x00000,
    Periodic = 0x20000,
    TscDeadline = 0x40000,
}

pub(in crate::arch::x86_64) fn get_apic_base() -> usize { unsafe { (read_from_msr(IA32_APIC_BASE as u32) & !0xFFF) as usize } }

pub(in crate::arch::x86_64) fn get_apic_flags() -> usize { unsafe { (read_from_msr(IA32_APIC_BASE as u32) & 0xFFF) as usize } }

pub fn send_eoi() { get_core_data().apic_mode.eoi(); }

pub trait ApicDriver {
    fn eoi(&self);
    fn id(&self) -> u32;
    fn timer_setup(&self, vector: u8, init_count: u32, mode: TimerMode);
    fn stop_timer(&self);
    fn current_count(&self) -> usize;
    fn arm_oneshot(&self, ticks: u32);
    fn send_ipi(&self, target_id: u32, vector: u32);
}

impl XApicDriver {
    pub fn init(&mut self) {
        unsafe {
            pic8259::disable();
            LAPIC_BASE_ADDR.get_or_init(|| get_apic_base());
            self.base_addr = get_apic_base() + *HHDMOFFSET;
            self.write_reg(SV_OFFSET, self.read_reg(SV_OFFSET) | (1 << 8) | 0xFF);
            self.write_reg(TPR_OFFSET, 0);
        }
    }

    pub(crate) unsafe fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u32;
            write_volatile(ptr, value);
        }
    }

    pub(crate) fn read_reg(&self, offset: usize) -> u32 {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u32;
            read_volatile(ptr)
        }
    }
}

impl ApicDriver for XApicDriver {
    fn eoi(&self) {
        unsafe {
            self.write_reg(EOI_OFFSET, 0);
        }
    }

    fn id(&self) -> u32 { self.read_reg(LAPIC_ID_OFFSET) }

    fn timer_setup(&self, vector: u8, init_count: u32, mode: TimerMode) {
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

    fn stop_timer(&self) { unsafe { self.write_reg(INIT_COUNT_OFFSET, 0) }; }

    fn current_count(&self) -> usize { self.read_reg(CURRENT_COUNT_OFFSET) as usize }

    fn arm_oneshot(&self, ticks: u32) {
        unsafe {
            self.write_reg(INIT_COUNT_OFFSET, ticks);
        }
    }

    fn send_ipi(&self, target_id: u32, vector: u32) {
        let lower = target_id << 24;
        unsafe {
            self.write_reg(0x310, lower);
            self.write_reg(0x300, vector | 0x4000);
        }
    }
}

impl X2ApicDriver {
    pub(crate) fn init(&mut self) {
        unsafe {
            pic8259::disable();

            // ensure x2apic is enabled on this core
            if !check_bit(get_apic_flags(), 10) {
                let newbase = get_apic_base() | get_apic_flags() | (1 << 10);
                write_to_msr(newbase as u64, IA32_APIC_BASE as u32);
            }

            LAPIC_BASE_ADDR.get_or_init(|| 0x800);
            self.base_addr = 0x800;
            self.write_reg(SV_OFFSET, self.read_reg(SV_OFFSET) | (1 << 8) | 0xFF);
            self.write_reg(TPR_OFFSET, 0);
        }
    }

    pub(crate) unsafe fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            write_to_msr(value as u64, (self.base_addr + (offset >> 4)) as u32);
        }
    }

    pub(crate) unsafe fn read_reg(&self, offset: usize) -> u32 { unsafe { read_from_msr((self.base_addr + (offset >> 4)) as u32) as u32 } }
}

impl ApicDriver for X2ApicDriver {
    fn eoi(&self) {
        unsafe {
            self.write_reg(EOI_OFFSET, 0);
        }
    }

    fn id(&self) -> u32 { unsafe { self.read_reg(LAPIC_ID_OFFSET) } }

    fn timer_setup(&self, vector: u8, init_count: u32, mode: TimerMode) {
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

    fn stop_timer(&self) { unsafe { self.write_reg(INIT_COUNT_OFFSET, 0) }; }

    fn current_count(&self) -> usize { unsafe { self.read_reg(CURRENT_COUNT_OFFSET) as usize } }

    fn arm_oneshot(&self, ticks: u32) {
        unsafe {
            self.write_reg(INIT_COUNT_OFFSET, ticks);
        }
    }

    fn send_ipi(&self, target_id: u32, vector: u32) {
        let val = ((target_id as u64) << 32) | 0x4000 | vector as u64;
        unsafe {
            write_to_msr(val, 0x830);
        }
    }
}

pub fn check_enable_x2apic() -> bool {
    if check_bit(get_apic_flags(), 10) {
        return true;
    }
    let feats = __cpuid(1).ecx;
    if check_bit(feats, 21) {
        let newbase = get_apic_base() | get_apic_flags() | (1 << 10);
        unsafe {
            write_to_msr(newbase as u64, IA32_APIC_BASE as u32);
        }
        return true;
    }
    false
}

/// (message_address_low, message_address_high, message_data)
/// for xapic: message_address_low is 0xfee0_0000 | (dest_apic_id << 12), high=0, data contains the vector in bits[0:7].
/// for x2apic: message_address_low is 0, message_address_high contains the full destination x2apic id, data contains the vector.
pub fn msi_message_fields_for_target(core_logical_id: usize, vector: u8) -> (u32, u32, u32) {
    let core = crate::arch::x86_64::cpu::core::arch_core_data_for(core_logical_id);
    msi_message_fields_for_lapic_id(core.lapic_id as u32, vector)
}

pub fn msi_message_fields_for_lapic_id(lapic_id: u32, vector: u8) -> (u32, u32, u32) {
    let msg_addr_low: u32 = 0xFEE0_0000u32 | ((lapic_id & 0xFF) << 12);
    (msg_addr_low, 0u32, vector as u32)
}
pub fn init_local_apic() -> ApicMode {
    if check_enable_x2apic() {
        let mut driver = X2ApicDriver { base_addr: 0 };
        driver.init();
        ApicMode::X2Apic(driver)
    } else {
        let mut driver = XApicDriver { base_addr: 0 };
        driver.init();
        ApicMode::XApic(driver)
    }
}

impl ApicDriver for ApicMode {
    fn eoi(&self) {
        match self {
            Self::XApic(a) => a.eoi(),
            Self::X2Apic(a) => a.eoi(),
        }
    }
    fn id(&self) -> u32 {
        match self {
            Self::XApic(a) => a.id(),
            Self::X2Apic(a) => a.id(),
        }
    }
    fn timer_setup(&self, vector: u8, init_count: u32, mode: TimerMode) {
        match self {
            Self::XApic(a) => a.timer_setup(vector, init_count, mode),
            Self::X2Apic(a) => a.timer_setup(vector, init_count, mode),
        }
    }
    fn stop_timer(&self) {
        match self {
            Self::XApic(a) => a.stop_timer(),
            Self::X2Apic(a) => a.stop_timer(),
        }
    }
    fn current_count(&self) -> usize {
        match self {
            Self::XApic(a) => a.current_count(),
            Self::X2Apic(a) => a.current_count(),
        }
    }
    fn arm_oneshot(&self, ticks: u32) {
        match self {
            Self::XApic(a) => a.arm_oneshot(ticks),
            Self::X2Apic(a) => a.arm_oneshot(ticks),
        }
    }
    fn send_ipi(&self, target_id: u32, vector: u32) {
        match self {
            Self::XApic(a) => a.send_ipi(target_id, vector),
            Self::X2Apic(a) => a.send_ipi(target_id, vector),
        }
    }
}

pub fn send_ipi_to_core(logical_id: usize, vector: u32) {
    let target = crate::arch::x86_64::cpu::core::arch_core_data_for(logical_id);
    get_core_data().apic_mode.send_ipi(target.lapic_id as u32, vector);
}

pub fn send_reschedule_ipi(logical_id: usize) {
    send_ipi_to_core(logical_id, RESCHEDULE_IPI_VECTOR);
}

pub fn send_tlb_shootdown_ipi(logical_id: usize) {
    send_ipi_to_core(logical_id, TLB_SHOOTDOWN_IPI_VECTOR);
}
