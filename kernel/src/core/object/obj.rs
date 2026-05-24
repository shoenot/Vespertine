use alloc::sync::Arc;
use core::fmt::Debug;

use crate::core::object::invoke::{
    Invocation,
    InvocationError,
};
use mnemosyne_abi::AccessRights;

pub trait KernelObject: Send + Sync + Debug {
    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError>;

    fn type_name(&self) -> &'static str { "Unknown" }
}

#[derive(Debug)]
pub struct HandleEntry {
    pub rights: AccessRights,
    pub object: Arc<dyn KernelObject>,
}

