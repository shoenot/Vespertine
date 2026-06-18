use vespertine_abi::{
    SYSTEM_USER,
    UserID,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credentials {
    user: UserID,
}

impl Credentials {
    pub const fn system() -> Self { Self { user: SYSTEM_USER } }

    pub const fn new(user: UserID) -> Self { Self { user } }

    pub const fn user(self) -> UserID { self.user }
}
