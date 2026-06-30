use core::arch::x86_64::__cpuid;
use core::ptr::{
    read_volatile,
    write_volatile,
};
use core::sync::atomic::{
    AtomicPtr,
    AtomicUsize,
    AtomicBool,
    Ordering,
};

use crate::x86_64::apic::pic8259;
use crate::x86_64::msr::{
    read_from_msr,
    write_to_msr,
};

use crate::x86_64::cpu::CpuLocalData;

use common::bitwise::check_bit;

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

const MAX_HAL_CORES: usize = 256;

static DIRECT_MAP_OFFSET: AtomicUsize = AtomicUsize::new(0);
static DIRECT_MAP_READY: AtomicBool = AtomicBool::new(false);
static CPU_LOCAL_DATA: [AtomicPtr<CpuLocalData>; MAX_HAL_CORES] = [const { AtomicPtr::new(core::ptr::null_mut()) }; MAX_HAL_CORES];

pub fn init_apic_direct_map(offset: usize) {
    DIRECT_MAP_OFFSET.store(offset, Ordering::Release);
    DIRECT_MAP_READY.store(true, Ordering::Release);
}

fn direct_map_offset() -> usize {
    assert!(DIRECT_MAP_READY.load(Ordering::Acquire), "HAL direct map offset was not initialized");
    DIRECT_MAP_OFFSET.load(Ordering::Acquire)
}

#[derive(Clone)]
pub struct XApicDriver {
    pub base_addr: usize,
}

#[derive(Clone)]
pub struct X2ApicDriver {
    pub base_addr: usize,
}

#[derive(Clone)]
pub enum LocalApic {
    XApic(XApicDriver),
    X2Apic(X2ApicDriver),
}

unsafe impl Send for XApicDriver {}
unsafe impl Sync for XApicDriver {}

unsafe impl Send for X2ApicDriver {}
unsafe impl Sync for X2ApicDriver {}

#[derive(Clone, Copy)]
pub enum TimerMode {
    OneShot = 0x00000,
    Periodic = 0x20000,
    TscDeadline = 0x40000,
}

pub trait LocalApicDriver {
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
            self.base_addr = get_apic_base() + direct_map_offset();
            self.write_reg(SV_OFFSET, self.read_reg(SV_OFFSET) | (1 << 8) | 0xFF);
            self.write_reg(TPR_OFFSET, 0);
        }
    }

    pub unsafe fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u32;
            write_volatile(ptr, value);
        }
    }

    pub fn read_reg(&self, offset: usize) -> u32 {
        unsafe {
            let ptr = (self.base_addr + offset) as *mut u32;
            read_volatile(ptr)
        }
    }
}

impl LocalApicDriver for XApicDriver {
    fn eoi(&self) {
        unsafe {
            self.write_reg(EOI_OFFSET, 0);
        }
    }

    fn id(&self) -> u32 { self.read_reg(LAPIC_ID_OFFSET) >> 24 }

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
    pub fn init(&mut self) {
        unsafe {
            pic8259::disable();

            // ensure x2apic is enabled on this core
            if !check_bit(get_apic_flags(), 10) {
                let newbase = get_apic_base() | get_apic_flags() | (1 << 10);
                write_to_msr(newbase as u64, IA32_APIC_BASE as u32);
            }
            self.base_addr = 0x800;
            self.write_reg(SV_OFFSET, self.read_reg(SV_OFFSET) | (1 << 8) | 0xFF);
            self.write_reg(TPR_OFFSET, 0);
        }
    }

    pub unsafe fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            write_to_msr(value as u64, (self.base_addr + (offset >> 4)) as u32);
        }
    }

    pub unsafe fn read_reg(&self, offset: usize) -> u32 { unsafe { read_from_msr((self.base_addr + (offset >> 4)) as u32) as u32 } }
}

impl LocalApicDriver for X2ApicDriver {
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

pub fn get_apic_base() -> usize { unsafe { (read_from_msr(IA32_APIC_BASE as u32) & !0xFFF) as usize } }

pub fn get_apic_flags() -> usize { unsafe { (read_from_msr(IA32_APIC_BASE as u32) & 0xFFF) as usize } }

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

pub(crate) fn register_cpu_local(logical_id: usize, data: *mut CpuLocalData) {
    assert!(logical_id < MAX_HAL_CORES, "Invalid Core ID");
    CPU_LOCAL_DATA[logical_id].store(data, Ordering::Release);
}

pub(crate) fn cpu_local_for(logical_id: usize) -> &'static CpuLocalData {
    assert!(logical_id < MAX_HAL_CORES, "Invalid Core ID");
    let ptr = CPU_LOCAL_DATA[logical_id].load(Ordering::Acquire);
    assert!(!ptr.is_null(), "Uninitialized HAL core");
    unsafe { &*ptr }
}

pub fn current_local_apic() -> &'static mut LocalApic {
    unsafe {
        &mut (*crate::x86_64::cpu::current_cpu_local()).local_apic
    }
}

pub fn send_eoi() {
    current_local_apic().eoi();
}

pub fn msi_message_fields_for_target(core_logical_id: usize, vector: u8) -> (u32, u32, u32) {
    let core = cpu_local_for(core_logical_id);
    match &core.local_apic {
        LocalApic::XApic(_) => msi_message_fields_for_xapic_id(core.hardware_id as u32, vector),
        LocalApic::X2Apic(_) => msi_message_fields_for_x2apic_id(core.hardware_id as u32, vector),
    }
}

pub fn msi_message_fields_for_xapic_id(apic_id: u32, vector: u8) -> (u32, u32, u32) {
    let msg_addr_low = 0xFEE0_0000u32 | ((apic_id & 0xFF) << 12);
    (msg_addr_low, 0, vector as u32)
}

pub fn msi_message_fields_for_x2apic_id(apic_id: u32, vector: u8) -> (u32, u32, u32) {
    let msg_addr_low = 0xFEE0_0000u32 | ((apic_id & 0xFF) << 12);
    let msg_addr_high = apic_id >> 8;
    (msg_addr_low, msg_addr_high, vector as u32)
}

pub fn init_local_apic() -> LocalApic {
    if check_enable_x2apic() {
        let mut driver = X2ApicDriver { base_addr: 0 };
        driver.init();
        LocalApic::X2Apic(driver)
    } else {
        let mut driver = XApicDriver { base_addr: 0 };
        driver.init();
        LocalApic::XApic(driver)
    }
}

impl LocalApicDriver for LocalApic {
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
    let target = cpu_local_for(logical_id);
    current_local_apic().send_ipi(target.hardware_id as u32, vector);
}

pub fn send_reschedule_ipi(logical_id: usize) {
    send_ipi_to_core(logical_id, RESCHEDULE_IPI_VECTOR);
}

pub fn send_tlb_shootdown_ipi(logical_id: usize) {
    send_ipi_to_core(logical_id, TLB_SHOOTDOWN_IPI_VECTOR);
}

pub fn arm_local_timer_oneshot(ticks: u32) {
    current_local_apic().arm_oneshot(ticks);
}

pub fn stop_local_timer() {
    current_local_apic().stop_timer();
}

pub fn setup_local_timer(vector: u8, init_count: u32, mode: TimerMode) {
    current_local_apic().timer_setup(vector, init_count, mode);
}
