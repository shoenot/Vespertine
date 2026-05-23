#![allow(dead_code)]

use core::mem::size_of;
use core::ptr::{
    null_mut,
    write_bytes,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use alloc::sync::Arc;

use crate::arch::get_core_data;
use crate::arch::x86_64::cpu::fpu::*;
use crate::arch::x86_64::interrupts::disable_interrupts;
use crate::kernel::object::models::process::Process;
use crate::kernel::sync::TicketLock;
use crate::kernel::thread::idle::*;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::{
    ThreadControlBlock,
    ThreadState,
    switch_threads_avx,
    switch_threads_legacy,
};
use crate::kernel::time::{
    get_time,
    ns_to_ticks, update_hardware_timer,
};
use crate::memory::paging::load_cr3;
use crate::{
    BOOTSTRAP_ALLOC, KERNEL_PROCESS, impl_queue_methods
};

pub static GLOBAL_TID: AtomicUsize = AtomicUsize::new(0);

pub const RFLAGS_IF: u64 = 0x202; // bit 9 is interrupt enable and bit 1 is always 1 (reserved)

pub const DEFAULT_QUANTUM: usize = 10_000_000;

pub static GRAVEYARD: TicketLock<TCBQueue> =
    TicketLock::new(TCBQueue { queue_length: AtomicUsize::new(0), head: null_mut(), tail: null_mut() });

pub struct TCBQueue {
    pub queue_length: AtomicUsize,
    head: *mut ThreadControlBlock,
    tail: *mut ThreadControlBlock,
}

unsafe impl Send for TCBQueue {}

impl_queue_methods!(TCBQueue, ThreadControlBlock, head, tail);

pub struct SchedulerState {
    pub core_logical_id: usize,

    pub queue_length: AtomicUsize,
    pub ready_queue_heads: [*mut ThreadControlBlock; 32],
    pub ready_queue_tails: [*mut ThreadControlBlock; 32],
    pub active_priorities: u32,

    pub sleep_queue_head: *mut ThreadControlBlock,
    pub mailbox: TicketLock<TCBQueue>,
    pub idle_thread: *mut ThreadControlBlock,

    pub current_thread: *mut ThreadControlBlock,
}

unsafe impl Send for SchedulerState {}
unsafe impl Sync for SchedulerState {}

pub fn get_new_tid() -> usize { GLOBAL_TID.fetch_add(1, Ordering::Relaxed) }

impl SchedulerState {
    pub const fn new() -> Self {
        SchedulerState {
            core_logical_id: 0,

            queue_length: AtomicUsize::new(0),
            ready_queue_heads: [null_mut(); 32],
            ready_queue_tails: [null_mut(); 32],
            active_priorities: 0,

            sleep_queue_head: null_mut(),
            mailbox: TicketLock::new(TCBQueue { queue_length: AtomicUsize::new(0), head: null_mut(), tail: null_mut() }),
            idle_thread: null_mut(),

            current_thread: null_mut(),
        }
    }

    pub fn init_basic(&mut self, logical_id: usize) {
        self.core_logical_id = logical_id;
    }

    pub fn init_threads(&mut self, logical_id: usize) {
        self.idle_thread = init_idle_thread(logical_id);

        let tcb_ptr = BOOTSTRAP_ALLOC.lock().alloc(size_of::<ThreadControlBlock>(), 8) as *mut ThreadControlBlock;

        unsafe { write_bytes(tcb_ptr as *mut u8, 0, size_of::<ThreadControlBlock>()) };

        let fpu_ptr = crate::arch::x86_64::task::context::allocate_fpu_context_bootstrap();

        unsafe {
            (*tcb_ptr).init(0, 0, 0, fpu_ptr, logical_id, ThreadPriority::MAXIMUM, KERNEL_PROCESS.clone());
            (*tcb_ptr).state = ThreadState::Running;
        }

        self.current_thread = tcb_ptr;
    }

    pub fn schedule(&mut self) {
        disable_interrupts();
        loop {
            let item = { self.mailbox.lock().pop() };
            if item.is_null() {
                break;
            }
            unsafe { (*item).state = ThreadState::Ready };
            self.push(item);
        }

        let mut next_thread = self.pop();
        let prev_thread = self.current_thread;
        if next_thread.is_null() {
            if !prev_thread.is_null() && unsafe { (*prev_thread).state == ThreadState::Running } {
                next_thread = prev_thread;
            } else {
                next_thread = self.idle_thread;
            }
        }

        // arm the timer for the next quantum if its not the idle thread.
        // idle only responds to interrupts.
        unsafe {
            if (*next_thread).priority != ThreadPriority::IDLE {
                (*next_thread).quantum_expiry = get_time() + ns_to_ticks(DEFAULT_QUANTUM);
            } else {
                (*next_thread).quantum_expiry = usize::MAX;
            }
        }

        if !prev_thread.is_null() {
            unsafe {
                if (*prev_thread).state == ThreadState::Running {
                    (*prev_thread).state = ThreadState::Ready;
                    if prev_thread != self.idle_thread && prev_thread != next_thread {
                        self.push(prev_thread);
                    }
                }
            }
        }

        self.current_thread = next_thread;
        unsafe {
            (*next_thread).state = ThreadState::Running;
        }

        let next_stack_top = unsafe { (*next_thread).stack_base + (*next_thread).stack_size };
        let core_data = get_core_data();
        core_data.core_gdt.tss.rsp[0] = next_stack_top as u64;

        core_data.kernel_rsp = next_stack_top;

        update_hardware_timer();

        if prev_thread == next_thread {
            return;
        }

        unsafe {
            let current_proc = &(*prev_thread).process;
            let next_proc = &(*next_thread).process;

            if !Arc::ptr_eq(current_proc, next_proc) {
                let next_pml4 = next_proc.vmm.read().get_pml4_addr();
                load_cr3(next_pml4 as u64);
            }
        }

        if !prev_thread.is_null() {
            unsafe {
                if USE_XSAVE.load(Ordering::Relaxed) {
                    switch_threads_avx(
                        &mut (*prev_thread).stack_ptr as *mut usize,
                        (*next_thread).stack_ptr,
                        (*prev_thread).extended_context,
                        (*next_thread).extended_context,
                    );
                } else {
                    switch_threads_legacy(
                        &mut (*prev_thread).stack_ptr as *mut usize,
                        (*next_thread).stack_ptr,
                        (*prev_thread).extended_context,
                        (*next_thread).extended_context,
                    );
                }
            }
        } else {
            let mut dummy_stack_ptr = 0usize;
            if USE_XSAVE.load(Ordering::Relaxed) {
                let dummy_fpu_ptr = gen_avx_dummy_fpu().ok().unwrap();
                unsafe {
                    switch_threads_avx(
                        &mut dummy_stack_ptr as *mut usize,
                        (*next_thread).stack_ptr,
                        dummy_fpu_ptr,
                        (*next_thread).extended_context,
                    );
                }
            } else {
                let mut dummy_fpu = LegacyXtCxt::new();
                unsafe {
                    let dummy_fpu_ptr = &mut dummy_fpu as *mut LegacyXtCxt as *mut u8;
                    switch_threads_legacy(
                        &mut dummy_stack_ptr as *mut usize,
                        (*next_thread).stack_ptr,
                        dummy_fpu_ptr,
                        (*next_thread).extended_context,
                    );
                }
            }
        }
    }

    pub fn push(&mut self, item: *mut ThreadControlBlock) {
        if item.is_null() {
            return;
        }
        let priority = unsafe { (*item).priority.as_usize() };
        unsafe {
            (*item).next = null_mut();
            if self.ready_queue_tails[priority].is_null() {
                self.ready_queue_heads[priority] = item;
                self.ready_queue_tails[priority] = item;
            } else {
                (*self.ready_queue_tails[priority]).next = item;
                self.ready_queue_tails[priority] = item;
            }
            self.queue_length.fetch_add(1, Ordering::Relaxed);
        }
        self.active_priorities |= 1 << priority;
    }

    pub fn pop(&mut self) -> *mut ThreadControlBlock {
        if self.active_priorities == 0 {
            return null_mut();
        }

        let highest_priority = self.active_priorities.trailing_zeros() as usize;
        let ret = self.ready_queue_heads[highest_priority];

        unsafe {
            if ret.is_null() {
                return null_mut();
            }

            self.ready_queue_heads[highest_priority] = (*ret).next;

            if self.ready_queue_heads[highest_priority].is_null() {
                self.ready_queue_tails[highest_priority] = null_mut();
            }

            if self.ready_queue_heads[highest_priority].is_null() {
                self.active_priorities &= !(1 << highest_priority);
            }

            (*ret).next = null_mut();
            self.queue_length.fetch_sub(1, Ordering::Relaxed);
            ret
        }
    }

    pub fn get_current_thread(&self) -> *mut ThreadControlBlock { self.current_thread }

    pub fn unblock(&mut self, thread: *mut ThreadControlBlock) {
        unsafe {
            (*thread).state = ThreadState::Ready;
        }
        self.push(thread);
    }

    pub fn terminate(&mut self) {
        unsafe {
            disable_interrupts();
            (*self.current_thread).state = ThreadState::Terminated;
            {
                GRAVEYARD.lock().push(self.current_thread);
            }
            self.schedule();
        }
    }
}
