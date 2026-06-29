use alloc::boxed::Box;

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    Invocation,
    ThreadOp,
};

use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::core::thread::dispatch::reschedule_thread_core;
use crate::core::thread::ThreadControlBlock;

#[derive(Debug)]
pub struct Thread {
    pub tcb: *mut ThreadControlBlock,
}

unsafe impl Sync for Thread {}
unsafe impl Send for Thread {}

#[async_trait]
impl KernelObject for Thread {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Thread(ThreadOp::Kill) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                unsafe {
                    (*self.tcb).request_cancel();
                }
                reschedule_thread_core(self.tcb);
                Ok(0)
            }
            Invocation::Thread(ThreadOp::Join) => Err(InvocationError::UnsupportedOperation),
            Invocation::Thread(ThreadOp::GetID) => {
                calling_rights.err_if_no(AccessRights::READ)?;
                let id = unsafe { (*self.tcb).thread_id };
                Ok(id)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str { "Thread" }
}
