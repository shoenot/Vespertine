use core::arch::asm;
use core::ops::{
    Deref,
    DerefMut,
};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, Ordering};

use super::gdt::*;
use crate::arch::x86_64::apic::lapic::ApicMode;
use crate::core::cpu::{KernelCoreData, MAX_CORES};
use crate::core::thread::dispatch::create_tcb;
use crate::core::thread::priority::ThreadPriority;
use crate::core::time::callout::timer_daemon;
use crate::util::write_to_msr;
use crate::{
    BOOTSTRAP_ALLOC,
    KERNEL_PROCESS,
};

const KERNEL_GS_BASE: u32 = 0xC0000101;

static ARCH_CPU_DATA: [AtomicPtr<CPULocalData>; MAX_CORES] = [const { AtomicPtr::new(null_mut()) }; MAX_CORES];

#[repr(C)]
pub struct CPULocalData {
    pub self_ptr: *mut CPULocalData,
    pub saved_user_rsp: usize, // offset 0x08
    pub kernel_rsp: usize,     // offset 0x10
    pub logical_id: usize,
    pub lapic_id: usize,
    pub core_gdt: CPULocalGDT,
    pub apic_mode: ApicMode,
    pub kernel_data: KernelCoreData,
}

impl Deref for CPULocalData {
    type Target = KernelCoreData;
    fn deref(&self) -> &Self::Target { &self.kernel_data }
}

impl DerefMut for CPULocalData {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.kernel_data }
}

pub fn init_core_data(lapic_id: usize, logical_id: usize, apic_mode: ApicMode) -> *mut CPULocalData {
    unsafe {
        let data_addr = BOOTSTRAP_ALLOC.lock().alloc(size_of::<CPULocalData>(), 8);
        let data_ptr = data_addr as *mut CPULocalData;

        let lgdt_ptr = &mut (*data_ptr).core_gdt as *mut CPULocalGDT;
        init_core_gdt(lgdt_ptr);

        (*data_ptr).self_ptr = data_ptr;
        (*data_ptr).logical_id = logical_id;
        (*data_ptr).lapic_id = lapic_id;
        (*data_ptr).apic_mode = apic_mode;
        core::ptr::write(&mut (*data_ptr).kernel_data, KernelCoreData::new(logical_id));

        data_ptr
    }
}

unsafe extern "sysv64" {
    pub(in crate::arch::x86_64::cpu) fn load_gdt(ptr: &GDTPointer);
}

pub fn activate_core(data_ptr: *mut CPULocalData) {
    unsafe {
        // load the gdt
        let gdt_ptr = (*data_ptr).core_gdt.gdt_ptr;
        load_gdt(&gdt_ptr);

        let data_addr = data_ptr as usize;
        // write GS
        write_to_msr(data_addr as u64, 0xC000_0100);
        write_to_msr(data_addr as u64, KERNEL_GS_BASE);

        init_syscall_msrs();
    }
}

pub fn get_core_data() -> &'static mut CPULocalData {
    let data_addr: u64;
    unsafe {
        asm!("mov {}, gs:[0]", out(reg) data_addr, options(nomem, nostack, preserves_flags));
        &mut *(data_addr as *mut CPULocalData)
    }
}


pub fn register_arch_core_data(logical_id: usize, data_ptr: *mut CPULocalData) {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    ARCH_CPU_DATA[logical_id].store(data_ptr, Ordering::Release);
}

pub fn arch_core_data_for(logical_id: usize) -> &'static CPULocalData {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    let ptr = ARCH_CPU_DATA[logical_id].load(Ordering::Acquire);
    assert!(!ptr.is_null(), "Uninitialized arch core");
    unsafe { &*ptr }
}

pub fn arch_core_data_for_mut(logical_id: usize) -> &'static mut CPULocalData {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    let ptr = ARCH_CPU_DATA[logical_id].load(Ordering::Acquire);
    assert!(!ptr.is_null(), "Uninitialized arch core");
    unsafe { &mut *ptr }
}

pub fn current_kernel_core_data() -> *mut KernelCoreData {
    &mut get_core_data().kernel_data as *mut KernelCoreData
}
