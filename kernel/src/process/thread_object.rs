use alloc::boxed::Box;

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    Invocation,
    ThreadOp,
};

use crate::object::help::RightsWrapper;
use crate::object::invoke::InvocationError;
use crate::object::obj::KernelObject;
use crate::sched::dispatch::reschedule_thread_core;
use crate::sched::Thread;

#[derive(Debug)]
pub struct ThreadObject {
    pub tcb: *mut Thread,
}

unsafe impl Sync for ThreadObject {}
unsafe impl Send for ThreadObject {}

#[async_trait]
impl KernelObject for ThreadObject {
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
