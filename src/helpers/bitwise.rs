use core::ops::{
    BitAnd,
    BitOr,
    Not,
    Shl,
};

#[inline]
pub fn set_bit<T>(value: T, bit: u8) -> T
where
    T: BitOr<T, Output = T> + Shl<u8, Output = T> + From<u8>,
{
    value | (T::from(1) << bit)
}

#[inline]
pub fn unset_bit<T>(value: T, bit: u8) -> T
where
    T: BitAnd<T, Output = T> + Shl<u8, Output = T> + From<u8> + Not<Output = T>,
{
    value & !(T::from(1) << bit)
}

#[inline]
pub fn check_bit<T>(value: T, bit: u8) -> bool
where
    T: BitAnd<T, Output = T> + Shl<u8, Output = T> + From<u8> + PartialEq<T> + Not<Output = T>,
{
    (value & (T::from(1) << bit)) != T::from(0)
}
