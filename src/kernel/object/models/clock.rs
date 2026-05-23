use crate::{kernel::{object::{invoke::{Invocation, InvocationError}, obj::KernelObject, op::ClockOp}, time::get_realtime}, klogln};

#[derive(Debug)]
pub struct Clock {}

impl KernelObject for Clock {
    fn type_name(&self) -> &'static str {
        "Clock"
    }

    fn invoke(&self, invocation: crate::kernel::object::invoke::Invocation, calling_rights: crate::kernel::object::handle::AccessRights) -> Result<usize, crate::kernel::object::invoke::InvocationError> {
        match invocation {
            Invocation::Clock(ClockOp::GetTimestamp) =>  { Ok(get_realtime() as usize) },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
