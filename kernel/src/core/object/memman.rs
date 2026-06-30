use alloc::boxed::Box;
use alloc::sync::Arc;

use async_trait::async_trait;
use vespertine_abi::op::MemManOp;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::mempool::MemPool;
use crate::core::object::obj::KernelObject;
use crate::process::current_process;

#[derive(Debug)]
pub struct MemoryManager;

#[async_trait]
impl KernelObject for MemoryManager {
    fn type_name(&self) -> &'static str { "Memory Manager" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::MemoryManager(MemManOp::CreatePool { limit }) => {
                calling_rights.err_if_no(AccessRights::CREATE)?;
                // 0 = unlimited
                let pool_limit = if limit == 0 { None } else { Some(limit) };
                let pool = Arc::new(MemPool::new(pool_limit, None));
                let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.handles.write().insert(pool, AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE);

                Ok(handle.0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
