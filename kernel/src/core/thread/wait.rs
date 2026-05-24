use core::ptr::null_mut;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::core::thread::ThreadControlBlock;
use crate::impl_queue_methods;

#[derive(Debug)]
pub struct WaitQueue {
    pub queue_length: AtomicUsize,
    head: *mut ThreadControlBlock,
    tail: *mut ThreadControlBlock,
}

impl WaitQueue {
    pub const fn new() -> Self { Self { queue_length: AtomicUsize::new(0), head: null_mut(), tail: null_mut() } }
}

impl_queue_methods!(WaitQueue, ThreadControlBlock, head, tail);
