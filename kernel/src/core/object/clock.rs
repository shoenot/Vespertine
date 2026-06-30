use alloc::boxed::Box;

use async_trait::async_trait;
use hal::usercopy::safe_copy_to;
use vespertine_abi::op::ClockOp;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::time::{
    get_realtime,
    sleep,
};

#[derive(Debug)]
pub struct Clock {}

#[async_trait]
impl KernelObject for Clock {
    fn type_name(&self) -> &'static str { "Clock" }

    async fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Clock(ClockOp::GetTimestamp { s_ptr, ns_ptr }) => {
                let (s, ns) = get_realtime();
                let s_src = &s as *const _ as *const u8;
                let ns_src = &ns as *const _ as *const u8;
                let len = size_of_val(&s);
                safe_copy_to(s_ptr as *mut u8, s_src, len);
                safe_copy_to(ns_ptr as *mut u8, ns_src, len);
                Ok(0)
            }
            Invocation::Clock(ClockOp::Sleep { ns }) => {
                sleep(ns);
                Ok(0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
