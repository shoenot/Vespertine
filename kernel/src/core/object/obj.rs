use alloc::boxed::Box;
use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use crate::core::object::invoke::InvocationError;

#[async_trait]
pub trait KernelObject: Send + Sync + Debug {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError>;

    fn type_name(&self) -> &'static str { "Unknown" }

    fn as_any(&self) -> &dyn core::any::Any { &() }
}

#[derive(Debug)]
pub struct HandleEntry {
    pub rights: AccessRights,
    pub object: Arc<dyn KernelObject>,
}
