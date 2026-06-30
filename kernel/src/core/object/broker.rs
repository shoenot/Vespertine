use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    BrokerOp,
    CapabilityID,
    Invocation,
};

use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::process::current_process;

#[derive(Debug)]
pub struct BrokerEntry {
    object: Arc<dyn KernelObject>,
    max_rights: AccessRights,
}

#[derive(Debug)]
pub struct Broker {
    capabilities: BTreeMap<CapabilityID, BrokerEntry>,
}

impl Broker {
    pub fn new() -> Self { Self { capabilities: BTreeMap::new() } }

    pub fn publish(&mut self, cap: CapabilityID, object: Arc<dyn KernelObject>, max_rights: AccessRights) {
        self.capabilities.insert(cap, BrokerEntry { object, max_rights });
    }
}

#[async_trait]
impl KernelObject for Broker {
    fn type_name(&self) -> &'static str { "Broker" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        calling_rights.err_if_no(AccessRights::READ)?;
        let Invocation::Broker(BrokerOp::Request { capability, requested_rights }) = invocation else {
            return Err(InvocationError::UnsupportedOperation);
        };

        let entry = self.capabilities.get(&capability).ok_or(InvocationError::InvalidArgument)?;

        // union with max rights and then check if the rights union is not empty
        let granted_rights = requested_rights & entry.max_rights;
        if granted_rights == AccessRights::new() {
            return Err(InvocationError::AccessDenied);
        }

        let caller = current_process().ok_or(InvocationError::InvalidHandle)?;

        let handle = caller.handles.write().insert(entry.object.clone(), granted_rights);

        Ok(handle.0)
    }
}
