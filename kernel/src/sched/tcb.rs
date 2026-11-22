use alloc::sync::Arc;
use core::ptr;
use core::sync::atomic::{
    AtomicBool,
    AtomicU8,
    AtomicUsize,
    Ordering,
};

use super::priority::ThreadPriority;
use crate::process::thread_object::ThreadObject;
use crate::process::{
    Process,
    ProcessControlBlock,
};
use crate::sync::TicketLock;
use crate::sched::block::{ThreadWakeRegistration, WakeTarget};
use crate::sched::scheduler::get_new_tid;
use crate::sched::wait::WaitQueue;

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
pub struct Thread {
    pub thread_id: usize,
    pub state: AtomicU8,
    pub handle: Option<Arc<ThreadObject>>,

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

    pub wake_target: Arc<WakeTarget>,
    pub reap_pins: AtomicUsize,
    pub process: Arc<ProcessControlBlock>,
    pub next: *mut Thread,
}

impl PartialEq for Thread {
    fn eq(&self, other: &Self) -> bool { self.thread_id == other.thread_id }
}

impl Thread {
    pub fn init(
        &mut self, stack_ptr: usize, stack_base: usize, stack_size: usize, fpu_ptr: *mut u8, assigned_core: usize,
        priority: ThreadPriority, proc: Process,
    ) {
        unsafe {
            let thread_ptr = ptr::from_mut(&mut *self);

            ptr::write(&mut self.thread_id, get_new_tid());
            ptr::write(&mut self.state, AtomicU8::new(ThreadState::Ready as u8));
            ptr::write(&mut self.handle, None);

            ptr::write(&mut self.cancel_requested, AtomicBool::new(false));

            ptr::write(&mut self.base_priority, priority);
            ptr::write(&mut self.effective_priority, priority);

            ptr::write(&mut self.wake_time, 0);
            ptr::write(&mut self.ready_since, 0);
            ptr::write(&mut self.total_runtime, 0);
            ptr::write(&mut self.last_started, 0);
            ptr::write(&mut self.quantum_expiry, 0);

            ptr::write(&mut self.stack_ptr, stack_ptr);
            ptr::write(&mut self.stack_base, stack_base);
            ptr::write(&mut self.stack_size, stack_size);
            ptr::write(&mut self.extended_context, fpu_ptr);
            ptr::write(&mut self.fs_base, 0);

            ptr::write(&mut self.assigned_core, AtomicUsize::new(assigned_core));
            ptr::write(&mut self.migration_disabled, AtomicBool::new(false));
            ptr::write(&mut self.block_state, TicketLock::new(ThreadBlockState::None));

            ptr::write(&mut self.wake_target, WakeTarget::new(thread_ptr));
            ptr::write(&mut self.reap_pins, AtomicUsize::new(0));
            ptr::write(&mut self.process, proc);
            ptr::write(&mut self.next, ptr::null_mut());
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

unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}

pub struct ThreadPin(*mut Thread);

impl ThreadPin {
    /// 'thread' must be reg'd and the caller must hold the registry lock/wake target lock
    /// so it can't be unregistered while this pin is acquired
    pub unsafe fn acquire_protected(thread: *mut Thread) -> Self {
        assert!(!thread.is_null());

        unsafe {
            (*thread)
                .reap_pins
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| { count.checked_add(1) })
                .expect("thread pin count overflow");
        }

        Self(thread)
    }

    pub fn as_ptr(&self) -> *mut Thread {
        self.0
    }
}

impl Drop for ThreadPin {
    fn drop(&mut self) {
        let previous = unsafe { (*self.0).reap_pins.fetch_sub(1, Ordering::Release) };
        assert_ne!(previous, 0, "thread pin count underflow");
    }
}
