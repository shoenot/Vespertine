use core::arch::asm;
use core::arch::x86_64::__cpuid;
use core::ptr::{
    read_volatile,
    write_volatile,
};

use super::pic8259;
use crate::kernel::sync::KernelOnceCell;
use crate::memory::HHDMOFFSET;
use crate::util::bitwise::check_bit;

const SV_OFFSET: usize = 0xF0;
const EOI_OFFSET: usize = 0xB0;
const TIMER_LVT_OFFSET: usize = 0x320;
const LAPIC_ID_OFFSET: usize = 0x20;
const TPR_OFFSET: usize = 0x80;
const DIVIDE_CONFIG_OFFSET: usize = 0x3E0;
const INIT_COUNT_OFFSET: usize = 0x380;
const CURRENT_COUNT_OFFSET: usize = 0x390;

const IA32_APIC_BASE: usize = 0x1B;

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

pub(in crate::arch::x86_64) fn get_apic_base() -> usize {
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

pub(in crate::arch::x86_64) fn get_apic_flags() -> usize {
    let (lower, upper): (u32, u32);
    unsafe {
        asm!("rdmsr", 
            in("ecx") IA32_APIC_BASE,
            out("eax") lower,
            out("edx") upper)
    }
    let base_phys = ((upper as u64) << 32) | (lower as u64);
    (base_phys & 0xFFF) as usize
}

pub(in crate::arch::x86_64) fn send_apic_eoi() {
    unsafe {
        let eoi_ptr = ((*LAPIC_BASE_ADDR + *HHDMOFFSET) + EOI_OFFSET) as *mut u32;
        write_volatile(eoi_ptr, 0);
    }
}

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
            self.write_reg(0x300, vector);
        }
    }
}

impl X2ApicDriver {
    pub(crate) fn init(&mut self) {
        unsafe {
            pic8259::disable();
            LAPIC_BASE_ADDR.get_or_init(|| 0x800);
            self.base_addr = 0x800;
            self.write_reg(SV_OFFSET, self.read_reg(SV_OFFSET) | (1 << 8) | 0xFF);
            self.write_reg(TPR_OFFSET, 0);
        }
    }

    pub(crate) unsafe fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            asm!("wrmsr",
                 in("ecx") self.base_addr + (offset >> 4),
                 in("edx") 0,
                 in("eax") value,
                 options(nomem, nostack, preserves_flags))
        }
    }

    pub(crate) unsafe fn read_reg(&self, offset: usize) -> u32 {
        let out: u32;
        unsafe {
            asm!("rdmsr",
                 in("ecx") self.base_addr + (offset >> 4),
                 out("edx") _,
                 out("eax") out,
                 options(nomem, nostack, preserves_flags))
        }
        out
    }
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
        unsafe {
            asm!("wrmsr",
                in("ecx") 0x830,
                in("edx") target_id,
                in("eax") vector,
            );
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
            asm!("wrmsr", 
                in("ecx") IA32_APIC_BASE,
                in("edx") (newbase >> 32) as u32,
                in("eax") newbase as u32,
                options(nomem, nostack, preserves_flags));
        }
        return true;
    }
    false
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
