use alloc::sync::Arc;
use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicU8,
    Ordering,
};

use super::priority::ThreadPriority;
use crate::KERNEL_PROCESS;
use crate::arch::get_core_data;
use crate::core::object::models::process::{
    Process,
    ProcessControlBlock,
};
use crate::core::thread::schedule::get_new_tid;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadState {
    Ready = 0,
    Running = 1,
    Blocked = 2,
    Terminated = 3,
}

impl ThreadState {
    pub fn from_raw(raw: u8) -> ThreadState {
        match raw {
            0 => ThreadState::Ready,
            1 => ThreadState::Running,
            2 => ThreadState::Blocked,
            3 => ThreadState::Terminated,
            _ => panic!("invalid thread state"),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct ThreadControlBlock {
    pub thread_id: usize,
    pub state: AtomicU8,
    pub priority: ThreadPriority,
    pub wake_time: usize,
    pub total_runtime: usize,
    pub quantum_expiry: usize,
    pub stack_ptr: usize,
    pub stack_base: usize,
    pub stack_size: usize,
    pub extended_context: *mut u8,
    pub fs_base: usize,
    pub home_core: usize,
    pub process: Arc<ProcessControlBlock>,
    pub next: *mut ThreadControlBlock,
}

impl PartialEq for ThreadControlBlock {
    fn eq(&self, other: &Self) -> bool { self.thread_id == other.thread_id }
}

impl ThreadControlBlock {
    pub fn init(
        &mut self, stack_ptr: usize, stack_base: usize, stack_size: usize, fpu_ptr: *mut u8, home_core: usize, priority: ThreadPriority,
        proc: Process,
    ) {
        unsafe {
            core::ptr::write(&mut self.thread_id, get_new_tid());
            core::ptr::write(&mut self.state, AtomicU8::new(ThreadState::Ready as u8));
            core::ptr::write(&mut self.priority, priority);
            core::ptr::write(&mut self.wake_time, 0);
            core::ptr::write(&mut self.total_runtime, 0);
            core::ptr::write(&mut self.quantum_expiry, 0);
            core::ptr::write(&mut self.stack_ptr, stack_ptr);
            core::ptr::write(&mut self.stack_base, stack_base);
            core::ptr::write(&mut self.stack_size, stack_size);
            core::ptr::write(&mut self.extended_context, fpu_ptr);
            core::ptr::write(&mut self.fs_base, 0);
            core::ptr::write(&mut self.home_core, home_core);
            core::ptr::write(&mut self.process, proc);
            core::ptr::write(&mut self.next, null_mut());
        }
    }

    pub fn state(&self) -> ThreadState { ThreadState::from_raw(self.state.load(Ordering::Acquire)) }

    pub fn set_state(&self, state: ThreadState) { self.state.store(state as u8, Ordering::Release); }

    pub fn transition(&self, old: ThreadState, new: ThreadState) -> Result<(), ThreadState> {
        self.state.compare_exchange(old as u8, new as u8, Ordering::AcqRel, Ordering::Acquire).map(|_| ()).map_err(ThreadState::from_raw)
    }
}

pub fn get_current_process<'a>() -> Option<&'a Process> {
    let thread = get_core_data().scheduler.get_current_thread();
    if thread.is_null() { KERNEL_PROCESS.get() } else { unsafe { Some(&(*thread).process) } }
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
