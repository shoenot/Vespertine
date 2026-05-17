use core::ops::{
    BitAnd,
    BitOr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandleID(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessRights(pub u8);

impl BitOr for AccessRights {
    type Output = Self;
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self::Output { Self(self.0 | rhs.0) }
}

impl BitAnd for AccessRights {
    type Output = Self;
    #[inline(always)]
    fn bitand(self, rhs: Self) -> Self::Output { Self(self.0 & rhs.0) }
}

impl AccessRights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const CREATE: Self = Self(1 << 3);
    pub const MUTATE: Self = Self(1 << 4);

    #[inline(always)]
    pub fn contains(&self, right: Self) -> bool { *self & right == right }

    #[inline(always)]
    pub fn union(&self, right: Self) -> Self { *self | right }
}
