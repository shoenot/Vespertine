use alloc::vec::Vec;
use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use crate::sched::Thread;
use crate::sched::dispatch::wake_thread;
use crate::impl_queue_methods;

#[derive(Debug)]
pub struct WaitQueue {
    pub queue_length: AtomicUsize,
    head: *mut Thread,
    tail: *mut Thread,
}

unsafe impl Send for WaitQueue {}

impl WaitQueue {
    pub const fn new() -> Self { Self { queue_length: AtomicUsize::new(0), head: null_mut(), tail: null_mut() } }

    pub fn remove(&mut self, target: *mut Thread) -> bool {
        if target.is_null() {
            return false;
        }

        let mut prev = null_mut() as *mut Thread;
        let mut cur = self.head;

        while !cur.is_null() {
            unsafe {
                let next = (*cur).next;

                if cur == target {
                    if prev.is_null() {
                        self.head = next;
                    } else {
                        (*prev).next = next;
                    }

                    if self.tail == cur {
                        self.tail = prev;
                    }

                    (*cur).next = null_mut();
                    self.queue_length.fetch_sub(1, Ordering::Relaxed);
                    return true;
                }
                prev = cur;
                cur = next;
            }
        }
        false
    }

    pub fn pop_wakeable(&mut self) -> *mut Thread {
        loop {
            let thread = self.pop();

            if thread.is_null() {
                return thread;
            }

            unsafe {
                (*thread).clear_block_state();
                return thread;
            }
        }
    }
}

impl_queue_methods!(WaitQueue, Thread, head, tail);

pub struct WakeToken {
    pub fired: AtomicBool,
    pub thread: *mut Thread,
}

unsafe impl Send for WakeToken {}
unsafe impl Sync for WakeToken {}

impl WakeToken {
    pub fn new(thread: *mut Thread) -> Self { Self { fired: AtomicBool::new(false), thread } }
}

#[derive(Debug)]
pub struct MultiWakeQueue {
    tokens: Vec<*mut WakeToken>,
}

unsafe impl Send for MultiWakeQueue {}

impl MultiWakeQueue {
    pub fn new() -> Self { Self { tokens: Vec::new() } }

    pub fn push(&mut self, token: *mut WakeToken) { self.tokens.push(token); }

    pub fn wake_all(&mut self) {
        for token_ptr in self.tokens.drain(..) {
            unsafe {
                let token = &*token_ptr;
                if token.fired.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    wake_thread(token.thread);
                }
            }
        }
    }

    pub fn remove(&mut self, token: *mut WakeToken) { self.tokens.retain(|&x| x != token); }
}
