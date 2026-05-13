use core::ptr::null_mut;

use crate::impl_queue_methods;
use crate::kernel::thread::ThreadControlBlock;

pub struct WaitQueue {
    head: *mut ThreadControlBlock,
    tail: *mut ThreadControlBlock,
}

impl WaitQueue {
    pub const fn new() -> Self { Self { head: null_mut(), tail: null_mut() } }
}

impl_queue_methods!(WaitQueue, ThreadControlBlock, head, tail);
