use core::sync::atomic::AtomicUsize;

use alloc::{collections::btree_map::BTreeMap, sync::Arc};
use crate::{kernel::{object::{handle::{HandleID, HandleTable}, obj::HandleEntry}, sync::RwLock}, memory::{ALLOCATOR, vmm::VirtMemManager}};

pub static GLOBAL_PID: AtomicUsize = AtomicUsize::new(0);

pub fn get_new_pid() -> usize {
    GLOBAL_PID.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

pub type Process = Arc<ProcessControlBlock>;

#[derive(Debug)]
pub struct ProcessControlBlock {
    pub proc_id: usize,
    pub proc_handles: RwLock<HandleTable>,
    pub vmm: RwLock<VirtMemManager>,
    // pub proc_threads: Vec<&ThreadControlBlock>,
}

impl ProcessControlBlock {
    pub fn new() -> Process {
        Arc::new(
            Self {
                proc_id: get_new_pid(),
                proc_handles: RwLock::new(HandleTable::new()),
                vmm: RwLock::new(VirtMemManager::new(&ALLOCATOR)),
            }
        )
    }
}
