use core::arch::asm;
use core::ops::{
    Deref,
    DerefMut,
};

use super::gdt::*;
use crate::{BOOTSTRAP_ALLOC, KERNEL_PROCESS, klogln};
use crate::arch::x86_64::apic::lapic::ApicMode;
use crate::kernel::cpu::KernelCoreData;
use crate::kernel::thread::dispatch::create_tcb;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::time::callout::timer_daemon;
use crate::util::bitwise::set_bit;
use crate::util::{
    read_from_msr,
    write_to_msr,
};

const KERNEL_GS_BASE: u32 = 0xC0000101;

#[repr(C)]
pub struct CPULocalData {
    pub self_ptr: *mut CPULocalData,
    pub saved_user_rsp: usize,  // offset 0x08
    pub kernel_rsp: usize,      // offset 0x10
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
        klogln!("core {} was allocated data addr 0x{:X}", logical_id, data_addr as usize);
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

pub fn init_timer_daemon(data_ptr: *mut CPULocalData) {
    unsafe {
        let data = &mut *data_ptr;
        data.timer_daemon_tcb = create_tcb(timer_daemon as *const () as usize, 0, 
            ThreadPriority::HIGH, KERNEL_PROCESS.clone()).unwrap();
        let timer_daemon_tcb = data.timer_daemon_tcb;
        data.scheduler.push(timer_daemon_tcb);
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
