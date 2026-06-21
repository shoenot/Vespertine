use vespertine_abi::AccessRights;

use crate::core::object::invoke::InvocationError;

pub trait RightsWrapper {
    fn err_if_no(&self, other: Self) -> Result<(), InvocationError>;
}

impl RightsWrapper for AccessRights {
    fn err_if_no(&self, other: Self) -> Result<(), InvocationError> {
        if !self.contains(other) {
            return Err(InvocationError::AccessDenied);
        }
        Ok(())
    }
}
