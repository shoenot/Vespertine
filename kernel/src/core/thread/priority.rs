use core::cmp::Ordering;
use core::ops::*;

#[derive(Eq, Copy, Clone, Debug)]
pub struct ThreadPriority(u8);

impl ThreadPriority {
    pub const MAXIMUM: ThreadPriority = ThreadPriority(0);
    pub const HIGH: ThreadPriority = ThreadPriority(4);
    pub const MEDIUM: ThreadPriority = ThreadPriority(8);
    pub const LOW: ThreadPriority = ThreadPriority(12);
    pub const REAPER: ThreadPriority = ThreadPriority(30);
    pub const IDLE: ThreadPriority = ThreadPriority(31);

    #[inline(always)]
    pub fn as_usize(&self) -> usize { self.0 as usize }
}

impl PartialEq for ThreadPriority {
    fn eq(&self, other: &Self) -> bool { self.0 == other.0 }
}

impl PartialOrd for ThreadPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.0.cmp(&other.0)) }
}

impl Ord for ThreadPriority {
    fn cmp(&self, other: &Self) -> Ordering { self.0.cmp(&other.0) }
}

macro_rules! impl_omni_integer_math {
    ($($t:ty),*) => {
        $(
            impl Add<$t> for ThreadPriority {
                type Output = Self;
                fn add(self, rhs: $t) -> Self::Output {
                    let calc = (self.0 as i64).saturating_add(rhs as i64);
                    ThreadPriority(calc.clamp(0, 31) as u8)
                }
            }

            impl Add<ThreadPriority> for $t {
                type Output = ThreadPriority;
                fn add(self, rhs: ThreadPriority) -> Self::Output {
                    let calc = (self as i64).saturating_add(rhs.0 as i64);
                    ThreadPriority(calc.clamp(0, 31) as u8)
                }
            }

            impl Sub<$t> for ThreadPriority {
                type Output = Self;
                fn sub(self, rhs: $t) -> Self::Output {
                    let calc = (self.0 as i64).saturating_sub(rhs as i64);
                    ThreadPriority(calc.clamp(0, 31) as u8)
                }
            }

            impl Sub<ThreadPriority> for $t {
                type Output = ThreadPriority;
                fn sub(self, rhs: ThreadPriority) -> Self::Output {
                    let calc = (self as i64).saturating_sub(rhs.0 as i64);
                    ThreadPriority(calc.clamp(0, 31) as u8)
                }
            }

            impl AddAssign<$t> for ThreadPriority {
                fn add_assign(&mut self, rhs: $t) {
                    let calc = (self.0 as i64).saturating_add(rhs as i64);
                    self.0 = calc.clamp(0, 31) as u8;
                }
            }

            impl SubAssign<$t> for ThreadPriority {
                fn sub_assign(&mut self, rhs: $t) {
                    let calc = (self.0 as i64).saturating_sub(rhs as i64);
                    self.0 = calc.clamp(0, 31) as u8;
                }
            }

            impl PartialEq<$t> for ThreadPriority {
                fn eq(&self, other: &$t) -> bool {
                    (self.0 as i64) == (*other as i64)
                }
            }

            impl PartialEq<ThreadPriority> for $t {
                fn eq(&self, other: &ThreadPriority) -> bool {
                    (*self as i64) == (other.0 as i64)
                }
            }

            impl PartialOrd<$t> for ThreadPriority {
                fn partial_cmp(&self, other: &$t) -> Option<Ordering> {
                    (self.0 as i64).partial_cmp(&(*other as i64))
                }
            }

            impl PartialOrd<ThreadPriority> for $t {
                fn partial_cmp(&self, other: &ThreadPriority) -> Option<Ordering> {
                    (*self as i64).partial_cmp(&(other.0 as i64))
                }
            }
        )*
    };
}

impl_omni_integer_math!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
