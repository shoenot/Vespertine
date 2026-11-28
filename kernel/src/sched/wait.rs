use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::sched::Thread;
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
}

impl_queue_methods!(WaitQueue, Thread, head, tail);
