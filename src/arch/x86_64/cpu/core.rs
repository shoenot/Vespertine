use alloc::collections::binary_heap::BinaryHeap;
use core::arch::asm;
use core::ptr::null_mut;

use super::gdt::*;
use crate::BOOTSTRAP_ALLOC;
use crate::arch::x86_64::apic::lapic::ApicMode;
use crate::kernel::sync::TicketLock;
use crate::kernel::thread::ThreadControlBlock;
use crate::kernel::thread::dispatch::create_tcb;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::schedule::SchedulerState;
use crate::kernel::thread::workqueue::WorkQueue;
use crate::kernel::time::callout::{
    Callout,
    timer_daemon,
};

const KERNEL_GS_BASE: u32 = 0xC0000101;

#[repr(C)]
pub struct CPULocalData {
    pub self_ptr: *mut CPULocalData,
    pub logical_id: usize,
    pub lapic_id: usize,
    pub core_gdt: CPULocalGDT,
    pub apic_mode: ApicMode,
    pub scheduler: SchedulerState,
    pub work_queue: WorkQueue,
    pub callout_queue: TicketLock<BinaryHeap<Callout>>,
    pub timer_daemon_tcb: *mut ThreadControlBlock,
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
        (*data_ptr).scheduler = SchedulerState::new();
        (*data_ptr).scheduler.init(logical_id);
        (*data_ptr).callout_queue = TicketLock::new(BinaryHeap::new());
        (*data_ptr).timer_daemon_tcb = null_mut();

        data_ptr
    }
}

pub fn init_timer_daemon(data_ptr: *mut CPULocalData) {
    unsafe {
        (*data_ptr).timer_daemon_tcb = create_tcb(timer_daemon as *const () as usize, 0, ThreadPriority::HIGH).unwrap();
        (*data_ptr).scheduler.push((*data_ptr).timer_daemon_tcb);
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
        asm!("wrmsr", 
            in("ecx") KERNEL_GS_BASE,
            in("edx") (data_addr >> 32) as u32,
            in("eax") data_addr as u32,
            options(nomem, nostack, preserves_flags));
    }
}

pub fn get_core_data() -> &'static mut CPULocalData {
    let data_addr: u64;
    unsafe {
        asm!("mov {}, gs:[0]", out(reg) data_addr, options(nomem, nostack, preserves_flags));
        &mut *(data_addr as *mut CPULocalData)
    }
}
