use core::sync::atomic::{AtomicUsize, Ordering};
use alloc::sync::Arc;
use hal::mmu::PageSize;

use crate::object::invoke::InvocationError;


#[derive(Debug)]
pub struct PoolState {
    limit: AtomicUsize,
    maximum_limit: usize,
    allocated: AtomicUsize,
    parent: Option<Arc<PoolState>>,
}

impl PoolState {
    pub fn new(limit: usize, maximum_limit: usize, parent: Option<Arc<Self>>) -> Self {
        assert!(limit <= maximum_limit);

        Self {
            limit: AtomicUsize::new(limit),
            maximum_limit,
            allocated: AtomicUsize::new(0),
            parent,
        }
    }

    pub fn reserve(self: &Arc<Self>, size: usize) -> Result<PoolReservation, InvocationError> {
        let bytes = size
            .div_ceil(PageSize::Size4K.bytes())
            .checked_mul(PageSize::Size4K.bytes())
            .ok_or(InvocationError::InvalidArgument)?;

        self.try_allocate(bytes)?;

        Ok(PoolReservation { pool: Arc::clone(self), bytes })
    }

    fn release(&self, bytes: usize) {
        let previous = self.allocated.fetch_sub(bytes, Ordering::SeqCst);
        debug_assert!(previous >= bytes, "pool reservation accounting underflow");

        if let Some(parent) = &self.parent {
            parent.release(bytes);
        }
    }
    pub fn try_allocate(&self, size: usize) -> Result<(), InvocationError> {
        let mut current = self.allocated.load(Ordering::Relaxed);
        loop {
            let requested = current.checked_add(size).ok_or(InvocationError::PoolExhausted)?;
            if requested > self.limit.load(Ordering::Acquire) {
                return Err(InvocationError::PoolExhausted);
            }
            match self.allocated.compare_exchange_weak(current, requested, Ordering::SeqCst, Ordering::Relaxed) {
                Ok(_) => break,                  // reservation success
                Err(actual) => current = actual, // retry bc another thread beat this
            }
        }

        if let Some(p) = &self.parent {
            if let Err(e) = p.try_allocate(size) {
                // if it didn't succeed we must roll back our local reservation
                self.allocated.fetch_sub(size, Ordering::SeqCst);
                return Err(e);
            }
        }

        Ok(())
    }

    pub fn request_expansion(&self, additional_bytes: usize) -> Result<usize, InvocationError> {
        if additional_bytes == 0 {
            return Err(InvocationError::InvalidArgument);
        }

        let mut current_limit = self.limit.load(Ordering::Acquire);
        loop {
            let new_limit = current_limit.checked_add(additional_bytes).ok_or(InvocationError::AccessDenied)?;
            if new_limit > self.maximum_limit {
                return Err(InvocationError::AccessDenied);
            }

            match self.limit.compare_exchange_weak(current_limit, new_limit, Ordering::SeqCst, Ordering::Acquire) {
                Ok(_) => return Ok(new_limit),
                Err(actual) => current_limit = actual,
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct PoolReservation {
    pool: Arc<PoolState>,
    bytes: usize,
}

impl Drop for PoolReservation {
    fn drop(&mut self) {
        self.pool.release(self.bytes);
    }
}
