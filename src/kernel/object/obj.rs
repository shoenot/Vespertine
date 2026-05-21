use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use core::fmt::{
    Debug,
    Display,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::kernel::object::handle::{
    AccessRights,
    HandleID,
};
use crate::kernel::object::invoke::{
    Invocation,
    InvocationError,
};

pub trait KernelObject: Send + Sync + Debug {
    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError>;

    fn type_name(&self) -> &'static str { "Unknown" }
}

#[derive(Debug)]
pub struct HandleEntry {
    pub rights: AccessRights,
    pub object: Arc<dyn KernelObject>,
}

