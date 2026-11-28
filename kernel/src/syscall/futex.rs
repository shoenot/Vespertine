use core::{future::poll_fn, sync::atomic::{AtomicBool, Ordering}, task::{Poll, Waker}};

use alloc::{collections::{btree_map::BTreeMap, vec_deque::VecDeque}, sync::Arc, vec::Vec};
use hal::usercopy::{copy_from_user, load_user_u32_nofault};

use crate::{executor::waiter::wake_all, memory::vmm::USER_SPACE_LIMIT, object::invoke::InvocationError, sync::TicketLock};

#[derive(Debug)]
struct Entry {
    selected: Arc<AtomicBool>,
    waker: Waker, 
}

#[derive(Debug)]
pub struct FutexTable {
    queues: TicketLock<BTreeMap<usize, VecDeque<Entry>>>,
}

struct Wait<'a> {
    table: &'a FutexTable,
    addr: usize, 
    selected: Arc<AtomicBool>,
    queued: bool,
}

impl FutexTable {
    pub fn new() -> Self {
        Self { queues: TicketLock::new(BTreeMap::new()) }
    }

    pub async fn wait(&self, addr: usize, expected: u32) -> Result<(), InvocationError> {
        validate_addr(addr)?;

        let mut wait = Wait {
            table: self,
            addr,
            selected: Arc::new(AtomicBool::new(false)),
            queued: false,
        };

        poll_fn(|cx| {
            if wait.queued {
                let mut queues = self.queues.lock();

                if wait.selected.load(Ordering::Acquire) { return Poll::Ready(Ok(())) };

                let entry = queues.get_mut(&addr).unwrap().iter_mut()
                    .find(|entry| Arc::ptr_eq(&entry.selected, &wait.selected))
                    .expect("unselected futex waiter missing from its queue");
                entry.waker.clone_from(cx.waker());
                return Poll::Pending
            }

            let mut queues = self.queues.lock();
            if let Some(value) = load_user_u32_nofault(addr) {
                if value != expected { return Poll::Ready(Ok(())) };
                queues.entry(addr).or_default().push_back(Entry { 
                    selected: Arc::clone(&wait.selected), 
                    waker: cx.waker().clone(), 
                });
                wait.queued = true;
                return Poll::Pending;
            }
            drop(queues);

            // let the vm resolve a demand fault without the queue lock
            // the bytes copied here are only used to establish access
            let mut scratch = [0u8; 4];
            if !copy_from_user(scratch.as_mut_ptr(), addr as *const u8, scratch.len()) {
                return Poll::Ready(Err(InvocationError::InvalidPointer));
            }

            cx.waker().wake_by_ref();
            Poll::Pending
        }).await
    }

    pub fn wake(&self, addr: usize, count: usize) -> Result<(), InvocationError> {
        validate_addr(addr)?;

        let wakers = {
            let mut queues = self.queues.lock();
            let Some(queue) = queues.get_mut(&addr) else { return Ok(()) };
            let mut wakers = Vec::new();

            for _ in 0..count.min(queue.len()) {
                let entry = queue.pop_front().unwrap();
                entry.selected.store(true, Ordering::Release);
                wakers.push(entry.waker);
            }

            if queue.is_empty() {
                queues.remove(&addr);
            }

            wakers
        };

        wake_all(wakers);
        Ok(())
    }
}

impl Drop for Wait<'_> {
    fn drop(&mut self) {
        if !self.queued || self.selected.load(Ordering::Acquire) { return };

        let mut queues = self.table.queues.lock();
        if let Some(queue) = queues.get_mut(&self.addr) {
            queue.retain(|entry| !Arc::ptr_eq(&entry.selected, &self.selected));
            if queue.is_empty() {
                queues.remove(&self.addr);
            }
        }
    }
}

fn validate_addr(addr: usize) -> Result<(), InvocationError> {
    if addr == 0 || addr.checked_add(3).is_none_or(|last| last >= USER_SPACE_LIMIT) {
        return Err(InvocationError::InvalidPointer);
    }
    if addr % 4 != 0 {
        return Err(InvocationError::InvalidArgument);
    }
    Ok(())
}
