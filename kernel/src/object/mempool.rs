use alloc::boxed::Box;
use alloc::sync::Arc;

use async_trait::async_trait;
use vespertine_abi::op::MemPoolOp;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use crate::memory::pool::PoolState;
use crate::memory::vmo::AnonymousVmo;
use crate::object::help::RightsWrapper;
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::object::io_queue::IoQueue;
use crate::object::vmo::VmoObject;
use crate::object::obj::KernelObject;
use crate::process::current_process;

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
        Self {
            state: Arc::new(PoolState::new(initial_limit, maximum_limit, parent)),
        }
    }

    pub fn allocate_backing(&self, size: usize) -> Result<Arc<AnonymousVmo>, InvocationError> {
        AnonymousVmo::new_in_pool(size, &self.state)
    }

    pub fn state(&self) -> &Arc<PoolState> {
        &self.state
    }
}

#[async_trait]
impl KernelObject for MemPool {
    fn type_name(&self) -> &'static str { "MemPool" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights, _ctx: InvokeContext<'_>) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::MemPool(MemPoolOp::AllocateVmo { size }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                let proc = current_process().ok_or(InvocationError::InvalidHandle)?;

                let backing = self.allocate_backing(size)?;
                let object = Arc::new(VmoObject::new(backing));
                let rights = AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE;
                let handle = proc.handles.write().insert(object, rights);

                Ok(handle.0)
            },
            Invocation::MemPool(MemPoolOp::CreateSubPool { limit }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                let sub_pool = Arc::new(MemPool::new(Some(limit), Some(self.state.clone())));

                let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.handles.write().insert(sub_pool, AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE);

                Ok(handle.0)
            },
            Invocation::MemPool(MemPoolOp::RequestExpansion { additional_bytes }) => {
                calling_rights.err_if_no(AccessRights::MUTATE)?;
                self.state.request_expansion(additional_bytes)
            },
            Invocation::MemPool(MemPoolOp::CreateIoQueue { entries }) => {
                calling_rights.err_if_no(AccessRights::CREATE)?;
                let process = current_process().ok_or(InvocationError::InvalidHandle)?;
                let queue = Arc::new(IoQueue::new(entries, self.state())?);
                let rights = AccessRights::READ | AccessRights::WRITE;
                Ok(process.handles.write().insert(queue, rights).0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
