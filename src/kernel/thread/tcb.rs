use core::ptr::null_mut;

use alloc::sync::Arc;

use super::priority::ThreadPriority;
use crate::kernel::{process::pcb::{Process, ProcessControlBlock}, thread::schedule::get_new_tid};

#[derive(Debug, PartialEq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

#[repr(C)]
#[derive(Debug)]
pub struct ThreadControlBlock {
    pub thread_id: usize,
    pub state: ThreadState,
    pub priority: ThreadPriority,
    pub wake_time: usize,
    pub total_runtime: usize,
    pub quantum_expiry: usize,
    pub stack_ptr: usize,
    pub stack_base: usize,
    pub stack_size: usize,
    pub extended_context: *mut u8,
    pub home_core: usize,
    pub process: Arc<ProcessControlBlock>,
    pub next: *mut ThreadControlBlock,
}

impl PartialEq for ThreadControlBlock {
    fn eq(&self, other: &Self) -> bool {
        self.thread_id == other.thread_id
    }
}

impl ThreadControlBlock {
    pub fn init(
        &mut self, stack_ptr: usize, stack_base: usize, stack_size: usize, fpu_ptr: *mut u8, home_core: usize, priority: ThreadPriority, proc: Process
    ) {
        self.thread_id = get_new_tid();
        self.state = ThreadState::Ready;
        self.priority = priority;
        self.total_runtime = 0;
        self.stack_ptr = stack_ptr;
        self.stack_base = stack_base;
        self.stack_size = stack_size;
        self.extended_context = fpu_ptr;
        self.home_core = home_core;
        self.process = proc;
        self.next = null_mut();
    }
}

unsafe extern "sysv64" {
    pub fn switch_threads_avx(
        old_stack_ptr: *mut usize, new_stack_ptr: usize, old_extended_context: *mut u8, new_extended_context: *const u8,
    );

    pub fn switch_threads_legacy(
        old_stack_ptr: *mut usize, new_stack_ptr: usize, old_extended_context: *mut u8, new_extended_context: *const u8,
    );
}

unsafe impl Send for ThreadControlBlock {}
unsafe impl Sync for ThreadControlBlock {}
