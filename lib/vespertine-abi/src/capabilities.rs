use crate::{
    AccessRights,
    HandleID,
};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityID(pub usize);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CapabilityGrant {
    pub id: HandleID,
    pub rights: AccessRights,
    pub capability: CapabilityID,
}
