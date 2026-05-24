use alloc::collections::binary_heap::BinaryHeap;
use alloc::vec::Vec;
use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicPtr,
    Ordering,
};

use limine::mp::MpGotoFunction;

use crate::arch::x86_64::cpu::core::{
    get_core_data,
    init_core_data,
    CPULocalData,
};
use crate::boot::smp::ap_entry;
use crate::boot::MP_REQUEST;
use crate::core::sync::{
    KernelOnceCell,
    TicketLock,
};
use crate::core::thread::schedule::SchedulerState;
use crate::core::thread::workqueue::WorkQueue;
use crate::core::thread::ThreadControlBlock;
use crate::core::time::callout::Callout;
use crate::klogln;
use crate::memory::magazine::Magazine;

#[repr(C)]
pub struct KernelCoreData {
    pub scheduler: SchedulerState,
    pub work_queue: WorkQueue,
    pub callout_queue: TicketLock<BinaryHeap<Callout>>,
    pub timer_daemon_tcb: *mut ThreadControlBlock,
    pub magazine: Magazine,
}

impl KernelCoreData {
    pub fn new(logical_id: usize) -> Self {
        let mut scheduler = SchedulerState::new();
        scheduler.init_basic(logical_id);
        Self {
            scheduler,
            work_queue: WorkQueue::new(),
            callout_queue: TicketLock::new(BinaryHeap::new()),
            timer_daemon_tcb: null_mut(),
            magazine: Magazine::init(),
        }
    }
}

pub const MAX_CORES: usize = 256;
pub static NUM_CORES: KernelOnceCell<usize> = KernelOnceCell::new();

static GLOBAL_CPU_DATA: [AtomicPtr<CPULocalData>; MAX_CORES] = [const { AtomicPtr::new(null_mut()) }; MAX_CORES];

pub fn register_core_data(logical_id: usize, data_ptr: *mut CPULocalData) {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    GLOBAL_CPU_DATA[logical_id].store(data_ptr, Ordering::Release);
}

pub fn init_smp() {
    let mp_resp = MP_REQUEST.response().expect("No SMP Response from limine");
    let bsp_id = mp_resp.bsp_lapic_id;
    register_core_data(0, get_core_data());

    let mut logical_id = 1;
    for core in mp_resp.cpus() {
        if core.lapic_id == bsp_id {
            continue;
        }

        let ap_data_ptr = init_core_data(core.lapic_id as usize, logical_id, get_core_data().apic_mode.clone());

        register_core_data(logical_id, ap_data_ptr);

        // let att = test_thread as *const ();
        // (*ap_data_ptr).scheduler.spawn(att as usize, core.processor_id as usize).unwrap();

        let ap_data_addr = ap_data_ptr as u64;
        let ap_entry_ptr = ap_entry as MpGotoFunction;

        core.bootstrap(ap_entry_ptr, ap_data_addr);

        logical_id += 1;
    }

    klogln!("Started all cores");

    NUM_CORES.get_or_init(|| logical_id);
}

pub fn get_core_data_for(logical_id: usize) -> &'static CPULocalData {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    let ptr = GLOBAL_CPU_DATA[logical_id].load(Ordering::Acquire);
    assert!(!ptr.is_null(), "Uninitialized core");
    unsafe { &mut *ptr }
}

pub fn try_get_core_data_for(logical_id: usize) -> Option<&'static CPULocalData> {
    assert!(logical_id < MAX_CORES, "Invalid Core ID");
    let ptr = GLOBAL_CPU_DATA[logical_id].load(Ordering::Acquire);
    if ptr.is_null() { None } else { Some(unsafe { &mut *ptr }) }
}

pub fn get_active_cores() -> Vec<u32> {
    let mut ret = Vec::new();
    for core in 0..*NUM_CORES {
        ret.push(get_core_data_for(core).lapic_id as u32);
    }
    ret
}
