use core::{ptr::{addr_of, write_volatile}, sync::atomic::{AtomicBool, AtomicUsize, Ordering}};

use crate::{KERNEL_PROCESS, arch::x86_64::task::syscall::safe_copy_to, kernel::{object::{handle::{AccessRights, HandleID, HandleTable}, invoke::{Invocation, InvocationError}, models::directory::Directory, obj::{HandleEntry, KernelObject}, op::ProcOp, vfs::{ROOT_DIRECTORY, kernel_duplicate, kernel_walk, proc_cpy_handle, proc_register_obj}}, program::load_elf, sync::RwLock, thread::{dispatch::spawn_user_thread, get_current_process, priority::ThreadPriority}}, memory::{ALLOCATOR, vmm::{VM_FLAG_USER, VM_FLAG_WRITE, VirtMemManager}}};
use alloc::sync::Arc;

pub static GLOBAL_PID: AtomicUsize = AtomicUsize::new(0);

pub fn get_new_pid() -> usize {
    GLOBAL_PID.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

pub type Process = Arc<ProcessControlBlock>;

pub struct ProcStatus {
    pub pid: usize,
    pub active_threads: usize,
    pub is_terminated: bool,
    pub memory_usage: usize,
}

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
    pub fn new(process_root: Arc<dyn KernelObject>, root_rights: AccessRights) -> Process {
        let proc = Arc::new(
            Self {
                proc_id: get_new_pid(),
                proc_handles: RwLock::new(HandleTable::new()),
                vmm: RwLock::new(VirtMemManager::new(&ALLOCATOR)),
                active_threads: AtomicUsize::new(0),
                is_terminated: AtomicBool::new(false),
            }
        );
        
        // new processes get root at handle 0, self_id at handle 1 
        let console_handle = kernel_walk("/Objects/ConsoleWriter", HandleID(0))
            .expect("Couldn't find console handle");
        let console_proc = KERNEL_PROCESS.proc_handles.read().resolve(console_handle, AccessRights::WRITE)
            .expect("Couldn't find console process");
        proc.proc_handles.write().insert_at(HandleID(0), process_root, root_rights);
        proc.proc_handles.write().insert_at(HandleID(1), proc.clone(), AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);
        proc.proc_handles.write().insert_at(HandleID(2), console_proc.clone(), root_rights);
        proc
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
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
