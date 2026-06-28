#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct UserID(pub u32);

pub const SYSTEM_USER: UserID = UserID(0);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnCredentials {
    Inherit,
    User { user: UserID },
}
