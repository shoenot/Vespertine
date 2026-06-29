use alloc::collections::binary_heap::BinaryHeap;
use alloc::vec::Vec;
use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicBool,
    AtomicPtr,
    AtomicUsize,
    Ordering,
};

use limine::mp::MpGotoFunction;

use crate::arch::x86_64::cpu::core::{
    CPULocalData,
    get_core_data,
    init_core_data,
};
use crate::boot::MP_REQUEST;
use crate::boot::smp::ap_entry;
use crate::core::sync::{
    KernelOnceCell,
    TicketLock,
};
use crate::core::thread::ThreadControlBlock;
use crate::core::thread::schedule::SchedulerState;
use crate::core::thread::workqueue::WorkQueue;
use crate::core::time::callout::Callout;
use crate::klogln;
use crate::memory::magazine::Magazine;

pub const NO_STEAL_REQUEST: usize = usize::MAX;

#[repr(C)]
pub struct KernelCoreData {
    pub logical_id: usize,
    pub scheduler: SchedulerState,
    pub work_queue: WorkQueue,
    pub callout_queue: TicketLock<BinaryHeap<Callout>>,
    pub timer_daemon_tcb: *mut ThreadControlBlock,
    pub timer_daemon_awoken: AtomicBool,
    pub magazine: Magazine,
    pub steal_requester: AtomicUsize,
    pub shootdown_generation: AtomicUsize,
}

impl KernelCoreData {
    pub fn new(logical_id: usize) -> Self {
        let mut scheduler = SchedulerState::new();
        scheduler.init_basic(logical_id);
        Self {
            logical_id,
            scheduler,
            work_queue: WorkQueue::new(),
            callout_queue: TicketLock::new(BinaryHeap::new()),
            timer_daemon_tcb: null_mut(),
            timer_daemon_awoken: AtomicBool::new(false),
            magazine: Magazine::init(),
            steal_requester: AtomicUsize::new(NO_STEAL_REQUEST),
            shootdown_generation: AtomicUsize::new(0),
        }
    }
}

pub const MAX_CORES: usize = 256;
pub static NUM_CORES: KernelOnceCell<usize> = KernelOnceCell::new();

static GLOBAL_CPU_DATA: [AtomicPtr<KernelCoreData>; MAX_CORES] = [const { AtomicPtr::new(null_mut()) }; MAX_CORES];

pub fn register_core_data(logical_id: usize, data_ptr: *mut KernelCoreData) {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    GLOBAL_CPU_DATA[logical_id].store(data_ptr, Ordering::Release);
}

pub fn init_smp() {
    let mp_resp = MP_REQUEST.response().expect("[FATAL] No SMP Response from limine");
    let bsp_id = mp_resp.bsp_lapic_id;
    let bsp = crate::arch::get_core_data();
    crate::arch::x86_64::cpu::core::register_arch_core_data(0, bsp);
    register_core_data(0, &mut bsp.kernel_data as *mut KernelCoreData);

    let mut logical_id = 1;
    for core in mp_resp.cpus() {
        if core.lapic_id == bsp_id {
            continue;
        }

        let ap_data_ptr = init_core_data(core.lapic_id as usize, logical_id, get_core_data().apic_mode.clone());
        crate::arch::x86_64::cpu::core::register_arch_core_data(logical_id, ap_data_ptr);

        unsafe {
            register_core_data(logical_id, &mut (*ap_data_ptr).kernel_data as *mut KernelCoreData);
        }

        let ap_data_addr = ap_data_ptr as u64;
        let ap_entry_ptr = ap_entry as MpGotoFunction;

        core.bootstrap(ap_entry_ptr, ap_data_addr);

        logical_id += 1;
    }

    klogln!("[SUCCESS] All CPUs started and operational.");

    NUM_CORES.get_or_init(|| logical_id);
}

pub fn get_core_data_for(logical_id: usize) -> &'static KernelCoreData {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    let ptr = GLOBAL_CPU_DATA[logical_id].load(Ordering::Acquire);
    assert!(!ptr.is_null(), "Uninitialized core");
    unsafe { &mut *ptr }
}

pub fn get_core_data_for_mut(logical_id: usize) -> &'static mut KernelCoreData {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    let ptr = GLOBAL_CPU_DATA[logical_id].load(Ordering::Acquire);
    assert!(!ptr.is_null(), "Uninitialized core");
    unsafe { &mut *ptr }
}

pub fn try_get_core_data_for(logical_id: usize) -> Option<&'static KernelCoreData> {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    let ptr = GLOBAL_CPU_DATA[logical_id].load(Ordering::Acquire);
    if ptr.is_null() { None } else { Some(unsafe { &mut *ptr }) }
}

pub fn get_active_cores() -> Vec<usize> {
    let mut ret = Vec::new();
    for core in 0..*NUM_CORES {
        ret.push(core);
    }
    ret
}

pub fn current_core() -> &'static KernelCoreData {
    let ptr = crate::arch::current_kernel_core_data();
    assert!(!ptr.is_null(), "Current core data was not initialized");
    unsafe { &*ptr }
}

pub fn current_core_mut() -> &'static mut KernelCoreData {
    let ptr = crate::arch::current_kernel_core_data();
    assert!(!ptr.is_null(), "Current core data was not initialized");
    unsafe { &mut *ptr }
}

pub fn current_core_id() -> usize {
    current_core().logical_id
}
