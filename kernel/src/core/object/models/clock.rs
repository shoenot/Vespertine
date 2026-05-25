use crate::core::{object::{invoke::{Invocation, InvocationError}, obj::KernelObject}, time::get_realtime};

use vespertine_abi::op::ClockOp;

#[derive(Debug)]
pub struct Clock {}

impl KernelObject for Clock {
    fn type_name(&self) -> &'static str {
        "Clock"
    }

    fn invoke(&self, invocation: crate::core::object::invoke::Invocation, _calling_rights: crate::core::object::handle::AccessRights) -> Result<usize, crate::core::object::invoke::InvocationError> {
        match invocation {
            Invocation::Clock(ClockOp::GetTimestamp) =>  { Ok(get_realtime() as usize) },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
