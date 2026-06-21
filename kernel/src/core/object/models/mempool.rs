use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use async_trait::async_trait;
use vespertine_abi::op::MemPoolOp;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::vmo::VmoObject;
use crate::core::object::obj::KernelObject;
use crate::core::thread::get_current_process;
use crate::memory::vmo::Vmo;
#[derive(Debug)]
pub struct PoolState {
    limit: AtomicUsize,
    maximum_limit: usize,
    allocated: AtomicUsize,
    parent: Option<Arc<PoolState>>,
}

impl PoolState {
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
pub struct MemPool {
    state: Arc<PoolState>,
}

impl MemPool {
    pub fn new(limit: Option<usize>, parent: Option<Arc<PoolState>>) -> Self {
        let limit = limit.unwrap_or(usize::MAX);
        Self::new_expandable(limit, limit, parent)
    }

    pub fn new_expandable(initial_limit: usize, maximum_limit: usize, parent: Option<Arc<PoolState>>) -> Self {
        assert!(initial_limit <= maximum_limit);
        Self {
            state: Arc::new(PoolState { limit: AtomicUsize::new(initial_limit), maximum_limit, allocated: AtomicUsize::new(0), parent }),
        }
    }
}

#[async_trait]
impl KernelObject for MemPool {
    fn type_name(&self) -> &'static str { "MemPool" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::MemPool(MemPoolOp::AllocateVmo { size }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.state.try_allocate(size)?;

                let vmo_arc = Vmo::new(size);
                let vmo_obj = Arc::new(VmoObject::new(vmo_arc));

                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.proc_handles.write().insert(vmo_obj, AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);

                Ok(handle.0)
            }
            Invocation::MemPool(MemPoolOp::CreateSubPool { limit }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                let sub_pool = Arc::new(MemPool::new(Some(limit), Some(self.state.clone())));

                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.proc_handles.write().insert(sub_pool, AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE);

                Ok(handle.0)
            }
            Invocation::MemPool(MemPoolOp::RequestExpansion { additional_bytes }) => {
                calling_rights.err_if_no(AccessRights::MUTATE)?;
                self.state.request_expansion(additional_bytes)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
