#![allow(dead_code)]

use alloc::alloc::{
    Layout,
    alloc,
};
use core::mem::size_of;
use core::ptr::{
    copy_nonoverlapping,
    null_mut,
    write_bytes,
    write_volatile,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::arch::x86_64::cpu::fpu::*;
use crate::arch::x86_64::interrupts::disable_interrupts;
use crate::arch::x86_64::task::context::*;
use crate::kernel::thread::idle::*;
use crate::kernel::thread::{
    ThreadControlBlock,
    ThreadError,
    ThreadState,
    switch_threads_avx,
    switch_threads_legacy,
};
use crate::kernel::time::arm_sleep_ns;
use crate::{
    BOOTSTRAP_ALLOC,
    impl_queue_methods,
};

pub static GLOBAL_TID: AtomicUsize = AtomicUsize::new(0);

pub const RFLAGS_IF: u64 = 0x202; // bit 9 is interrupt enable and bit 1 is always 1 (reserved)

pub const DEFAULT_QUANTUM: usize = 10_000_000;

pub struct SchedulerState {
    pub ready_queue_head: *mut ThreadControlBlock,
    pub ready_queue_tail: *mut ThreadControlBlock,
    pub current_thread: *mut ThreadControlBlock,
    pub idle_thread: *mut ThreadControlBlock,
    pub sleep_queue_head: *mut ThreadControlBlock,
}

unsafe impl Send for SchedulerState {}
unsafe impl Sync for SchedulerState {}

pub fn get_new_tid() -> usize { GLOBAL_TID.fetch_add(1, Ordering::Relaxed) }

impl_queue_methods!(SchedulerState, ThreadControlBlock, ready_queue_head, ready_queue_tail);

impl SchedulerState {
    pub const fn new() -> Self {
        SchedulerState {
            ready_queue_head: null_mut(),
            ready_queue_tail: null_mut(),
            current_thread: null_mut(),
            idle_thread: null_mut(),
            sleep_queue_head: null_mut(),
        }
    }

    pub fn init(&mut self) {
        self.idle_thread = init_idle_thread();

        let tcb_ptr = BOOTSTRAP_ALLOC.lock().alloc(size_of::<ThreadControlBlock>(), 8) as *mut ThreadControlBlock;

        unsafe { write_bytes(tcb_ptr as *mut u8, 0, size_of::<ThreadControlBlock>()) };

        let fpu_ptr = if USE_XSAVE.load(Ordering::Relaxed) {
            let size = FPU_CXT_SIZE.load(Ordering::Relaxed);
            let fpu_ptr = BOOTSTRAP_ALLOC.lock().alloc(size, 64) as *mut u8;
            let def = CLEAN_FPU_CXT.load(Ordering::Relaxed);
            unsafe { copy_nonoverlapping(def, fpu_ptr, size) };
            fpu_ptr
        } else {
            let fpu_size = FPU_CXT_SIZE.load(Ordering::Relaxed);
            let fpu_ptr = BOOTSTRAP_ALLOC.lock().alloc(fpu_size, 16);
            fpu_ptr as *mut u8
        };

        unsafe {
            (*tcb_ptr).init(0, 0, fpu_ptr);
            (*tcb_ptr).state = ThreadState::Running;
        }

        self.current_thread = tcb_ptr;
    }

    pub fn spawn(&mut self, entry_point: usize, arg: usize) -> Result<(), ThreadError> {
        let stack_size = 4096 * 4;
        let fpu_size = FPU_CXT_SIZE.load(Ordering::Relaxed);
        // alloc memory for structs
        let tcb_layout = Layout::new::<ThreadControlBlock>();
        let stack_layout = Layout::from_size_align(stack_size, 4096)?;

        let tcb_ptr = unsafe { alloc(tcb_layout) as *mut ThreadControlBlock };
        let stack_base = unsafe { alloc(stack_layout) as usize };

        unsafe {
            let stack_ptr_u64 = stack_base as *mut u64;
            for i in 0..(stack_size / 8) {
                write_volatile(stack_ptr_u64.add(i), 0);
            }

            write_volatile(tcb_ptr as *mut u8, 0);
        }

        // init extended context state
        let fpu_ptr = if USE_XSAVE.load(Ordering::Relaxed) {
            gen_avx_dummy_fpu()?
        } else {
            let fpu_layout = Layout::from_size_align(fpu_size, 16)?;
            let fpu_ptr = unsafe { alloc(fpu_layout) as *mut u8 };
            let def = CLEAN_LEGACY_FPU_CXT.lock();
            let default_fpu_ref = def.as_ref().expect("Clean FPU not initialized");
            unsafe { copy_nonoverlapping(default_fpu_ref as *const LegacyXtCxt, fpu_ptr as *mut LegacyXtCxt, 1) };
            fpu_ptr as *mut u8
        };

        let stack_top = stack_base + stack_size;
        let context_addr = stack_top - size_of::<ThreadContext>();
        let context_addr = context_addr & !0xF; // align to 16 bytes
        let context = unsafe { &mut *(context_addr as *mut ThreadContext) };

        context.init(entry_point as u64, (stack_top - 8) as u64, arg);

        let switch_addr = context_addr - size_of::<SwitchContext>();
        let switch_context = unsafe { &mut *(switch_addr as *mut SwitchContext) };

        unsafe extern "C" {
            fn thread_entry_stub();
        }
        switch_context.init((thread_entry_stub as *const ()) as usize);

        // init TCB
        unsafe {
            (*tcb_ptr).init(switch_addr, stack_base, fpu_ptr);
        }

        // push new tcb to queue
        self.push(tcb_ptr);

        Ok(())
    }

    pub fn schedule(&mut self) {
        let mut next_thread = self.pop();
        if next_thread.is_null() {
            next_thread = self.idle_thread;
        }

        let prev_thread = self.current_thread;

        if prev_thread == self.idle_thread && next_thread == self.idle_thread {
            return;
        }

        if !prev_thread.is_null() {
            unsafe {
                if (*prev_thread).state == ThreadState::Running {
                    (*prev_thread).state = ThreadState::Ready;
                    if prev_thread != self.idle_thread {
                        self.push(prev_thread);
                    }
                }
            }
        }

        self.current_thread = next_thread;
        unsafe {
            (*next_thread).state = ThreadState::Running;
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

    pub fn push_sleep(&mut self, tcb: *mut ThreadControlBlock) {
        unsafe {
            (*tcb).next = null_mut();

            if self.sleep_queue_head.is_null() || (*tcb).wake_time < (*self.sleep_queue_head).wake_time {
                (*tcb).next = self.sleep_queue_head;
                self.sleep_queue_head = tcb;
                return;
            }

            let mut current = self.sleep_queue_head;
            while !(*current).next.is_null() && (*(*current).next).wake_time <= (*tcb).wake_time {
                current = (*current).next;
            }

            (*tcb).next = (*current).next;
            (*current).next = tcb;
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
            arm_sleep_ns(DEFAULT_QUANTUM);
            self.schedule();
        }
    }
}

