#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct UserID(pub u32);

pub const SYSTEM_USER: UserID = UserID(0);
