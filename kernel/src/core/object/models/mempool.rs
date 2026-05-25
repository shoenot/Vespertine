use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;

use crate::{core::{object::{invoke::InvocationError, models::vmo::VmoObject, obj::KernelObject}, thread::get_current_process}, memory::vmo::Vmo};
use vespertine_abi::Invocation;

use vespertine_abi::op::MemPoolOp;
use vespertine_abi::AccessRights;
#[derive(Debug)]
pub struct PoolState {
    limit: Option<usize>,
    allocated: AtomicUsize,
    parent: Option<Arc<PoolState>>,
}

impl PoolState {
    pub fn try_allocate(&self, size: usize) -> Result<(), InvocationError> {
        let mut current = self.allocated.load(Ordering::Relaxed);
        loop {
            if let Some(lim) = self.limit {
                if current + size > lim {
                    return Err(InvocationError::BufferFull);
                }
            }
            match self.allocated.compare_exchange_weak(
                current, 
                current + size,
                Ordering::SeqCst,
                Ordering::Relaxed
            ) {
                Ok(_) => break, // reservation success 
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
}

#[derive(Debug)]
pub struct MemPool {
    state: Arc<PoolState>,
}

impl MemPool {
    pub fn new(limit: Option<usize>, parent: Option<Arc<PoolState>>) -> Self {
        Self {
            state: Arc::new(PoolState { 
                limit, 
                allocated: AtomicUsize::new(0),
                parent,
            })
        }
    }
}

impl KernelObject for MemPool {
    fn type_name(&self) -> &'static str {
        "MemPool"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::MemPool(MemPoolOp::AllocateVmo { size }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                
                self.state.try_allocate(size)?;

                let vmo_arc = Vmo::new(size);
                let vmo_obj = Arc::new(VmoObject::new(vmo_arc));

                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.proc_handles.write().insert(
                    vmo_obj,
                    AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE
                );

                Ok(handle.0)
            },
            Invocation::MemPool(MemPoolOp::CreateSubPool { limit }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
             
                let sub_pool = Arc::new(MemPool::new(
                        Some(limit),
                        Some(self.state.clone())
                ));

                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.proc_handles.write().insert(
                    sub_pool,
                    AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE
                );

                Ok(handle.0)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
