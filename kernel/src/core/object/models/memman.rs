use alloc::sync::Arc;

use crate::core::{object::{invoke::{Invocation, InvocationError}, models::mempool::MemPool, obj::KernelObject}, thread::get_current_process};

use mnemosyne_abi::{HandleID, AccessRights};
use mnemosyne_abi::op::MemManOp;

#[derive(Debug)]
pub struct MemoryManager;

impl KernelObject for MemoryManager {
    fn type_name(&self) -> &'static str {
        "Memory Manager"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::MemoryManager(MemManOp::CreatePool { limit }) => {
                if !calling_rights.contains(AccessRights::CREATE) {
                    return Err(InvocationError::AccessDenied);
                }

                // 0 = unlimited
                let pool_limit = if limit == 0 { None } else { Some(limit) };
                let pool = Arc::new(MemPool::new(pool_limit, None));
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = proc.proc_handles.write().insert(
                    pool,
                    AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE
                );

                Ok(handle.0)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
