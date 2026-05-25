use core::{ptr::addr_of, sync::atomic::{AtomicBool, AtomicUsize, Ordering}};

use crate::{arch::x86_64::task::syscall::safe_copy_to, core::{object::{handle::HandleTable, invoke::InvocationError, obj::KernelObject}, sync::RwLock}, memory::{vmm::VirtMemManager, ALLOCATOR}};
use vespertine_abi::Invocation;
use alloc::sync::Arc;

use vespertine_abi::op::ProcOp;
use vespertine_abi::ProcStatus;
use vespertine_abi::{AccessRights, HandleID};

pub static GLOBAL_PID: AtomicUsize = AtomicUsize::new(0);

pub fn get_new_pid() -> usize {
    GLOBAL_PID.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

pub type Process = Arc<ProcessControlBlock>;

#[repr(C)]
#[derive(Debug)]
pub struct ProcessControlBlock {
    pub proc_id: usize,
    pub proc_handles: RwLock<HandleTable>,
    pub vmm: RwLock<VirtMemManager>,
    pub active_threads: AtomicUsize,
    pub is_terminated: AtomicBool,
}

impl ProcessControlBlock {
    pub fn new(init_table: HandleTable) -> Process {
        Arc::new(
            Self {
                proc_id: get_new_pid(),
                proc_handles: RwLock::new(init_table),
                vmm: RwLock::new(VirtMemManager::new(&ALLOCATOR)),
                active_threads: AtomicUsize::new(0),
                is_terminated: AtomicBool::new(false),
            }
        )
    }

    pub fn status(&self, ptr: *mut ProcStatus) -> Result<usize, InvocationError> {
        let proc_status = ProcStatus { 
            pid: self.proc_id,
            active_threads: self.active_threads.load(Ordering::Relaxed),
            is_terminated: self.is_terminated.load(Ordering::Relaxed),
            memory_usage: self.vmm.read().get_total_allocated_size(),
        };
        let src_ptr = addr_of!(proc_status) as *const u8;
        safe_copy_to(ptr as *mut u8, src_ptr, size_of::<ProcStatus>());
        Ok(0)
    }
}

impl KernelObject for ProcessControlBlock {
    fn type_name(&self) -> &'static str {
        "Process"
    }

    fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Proc(ProcOp::Kill) => { self.is_terminated.store(true, Ordering::SeqCst); Ok(0) },
            Invocation::Proc(ProcOp::GetStatus { status_ptr }) => self.status(status_ptr),
            Invocation::Proc(ProcOp::Unmap { vaddr, len } ) => {
                self.vmm.write().munmap(vaddr, len).map(|_| 0).map_err(|_| InvocationError::InvalidArgument)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
