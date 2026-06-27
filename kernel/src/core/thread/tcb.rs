use alloc::sync::Arc;
use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicBool,
    AtomicU8,
    AtomicUsize,
    Ordering,
};

use super::priority::ThreadPriority;
use crate::KERNEL_PROCESS;
use crate::arch::get_core_data;
use crate::core::object::models::process::{
    Process,
    ProcessControlBlock,
};
use crate::core::sync::TicketLock;
use crate::core::thread::block::ThreadWakeRegistration;
use crate::core::thread::schedule::get_new_tid;
use crate::core::thread::wait::WaitQueue;

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

#[derive(Clone, Debug)]
pub enum ThreadBlockState {
    None,
    WaitQueue { queue: *const TicketLock<WaitQueue> },
    Registration { registration: Arc<ThreadWakeRegistration> },
    Futex { addr: usize },
}

unsafe impl Send for ThreadBlockState {}
unsafe impl Sync for ThreadBlockState {}

#[repr(C)]
#[derive(Debug)]
pub struct ThreadControlBlock {
    pub thread_id: usize,
    pub state: AtomicU8,

    pub cancel_requested: AtomicBool,

    pub base_priority: ThreadPriority,
    pub effective_priority: ThreadPriority,

    pub wake_time: usize,
    pub ready_since: usize,
    pub total_runtime: usize,
    pub last_started: usize,
    pub quantum_expiry: usize,

    pub stack_ptr: usize,
    pub stack_base: usize,
    pub stack_size: usize,
    pub extended_context: *mut u8,
    pub fs_base: usize,

    pub assigned_core: AtomicUsize,
    pub migration_disabled: AtomicBool,
    pub block_state: TicketLock<ThreadBlockState>,

    pub process: Arc<ProcessControlBlock>,
    pub next: *mut ThreadControlBlock,
}

impl PartialEq for ThreadControlBlock {
    fn eq(&self, other: &Self) -> bool { self.thread_id == other.thread_id }
}

impl ThreadControlBlock {
    pub fn init(
        &mut self, stack_ptr: usize, stack_base: usize, stack_size: usize, fpu_ptr: *mut u8, assigned_core: usize,
        priority: ThreadPriority, proc: Process,
    ) {
        unsafe {
            core::ptr::write(&mut self.thread_id, get_new_tid());
            core::ptr::write(&mut self.state, AtomicU8::new(ThreadState::Ready as u8));

            core::ptr::write(&mut self.cancel_requested, AtomicBool::new(false));

            core::ptr::write(&mut self.base_priority, priority);
            core::ptr::write(&mut self.effective_priority, priority);

            core::ptr::write(&mut self.wake_time, 0);
            core::ptr::write(&mut self.ready_since, 0);
            core::ptr::write(&mut self.total_runtime, 0);
            core::ptr::write(&mut self.last_started, 0);
            core::ptr::write(&mut self.quantum_expiry, 0);

            core::ptr::write(&mut self.stack_ptr, stack_ptr);
            core::ptr::write(&mut self.stack_base, stack_base);
            core::ptr::write(&mut self.stack_size, stack_size);
            core::ptr::write(&mut self.extended_context, fpu_ptr);
            core::ptr::write(&mut self.fs_base, 0);

            core::ptr::write(&mut self.assigned_core, AtomicUsize::new(assigned_core));
            core::ptr::write(&mut self.migration_disabled, AtomicBool::new(false));
            core::ptr::write(&mut self.block_state, TicketLock::new(ThreadBlockState::None));

            core::ptr::write(&mut self.process, proc);
            core::ptr::write(&mut self.next, null_mut());
        }
    }

    pub fn state(&self) -> ThreadState { ThreadState::from_raw(self.state.load(Ordering::Acquire)) }

    pub fn set_state(&self, state: ThreadState) { self.state.store(state as u8, Ordering::Release); }

    pub fn transition(&self, old: ThreadState, new: ThreadState) -> Result<(), ThreadState> {
        self.state.compare_exchange(old as u8, new as u8, Ordering::AcqRel, Ordering::Acquire).map(|_| ()).map_err(ThreadState::from_raw)
    }

    pub fn assigned_core(&self) -> usize { self.assigned_core.load(Ordering::Acquire) }

    pub fn set_assigned_core(&self, core: usize) { self.assigned_core.store(core, Ordering::Release); }

    pub fn is_migratable(&self) -> bool { !self.migration_disabled.load(Ordering::Acquire) }

    pub fn pin_to_core(&self, core: usize) {
        self.assigned_core.store(core, Ordering::Release);
        self.migration_disabled.store(true, Ordering::Release);
    }

    pub fn request_cancel(&self) { self.cancel_requested.store(true, Ordering::Release); }

    pub fn cancel_requested(&self) -> bool { self.cancel_requested.load(Ordering::Acquire) }

    pub fn set_block_state(&self, state: ThreadBlockState) { *self.block_state.lock() = state; }

    pub fn take_block_state(&self) -> ThreadBlockState { core::mem::replace(&mut *self.block_state.lock(), ThreadBlockState::None) }

    pub fn clear_block_state(&self) { *self.block_state.lock() = ThreadBlockState::None; }
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
