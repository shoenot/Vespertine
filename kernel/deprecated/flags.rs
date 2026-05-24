
#[repr(transparent)]
#[derive(Copy, Clone, Eq, PartialEq)]
pub struct PgFrameFlags(pub u16);

impl PgFrameFlags {
    pub const PF_FREE           : u16 = 1 << 0;
    pub const PF_KERNEL         : u16 = 1 << 1;
    pub const PF_PAGE_TABLE     : u16 = 1 << 2;
    pub const PF_VMO            : u16 = 1 << 3;
    pub const PF_PINNED         : u16 = 1 << 4;
    pub const PF_BUDDY_HEAD     : u16 = 1 << 5;

    pub const BUDDY_ORDER_MASK  : u16 = 0xFF00;

    #[inline(always)]
    pub const fn new(flags: u16, order: u8) -> Self {
        Self((flags & !Self::BUDDY_ORDER_MASK) | ((order as u16) << 8))
    }

    #[inline(always)]
    pub const fn has_flag(&self, flag: u16) -> bool {
        self.0 & flag != 0
    }

    #[inline(always)]
    pub const fn with_flag(&self, flag: u16) -> Self {
        Self(self.0 | (flag & !Self::BUDDY_ORDER_MASK))
    }

    #[inline(always)]
    pub const fn without_flag(&self, flag: u16) -> Self {
        Self(self.0 & !(flag & !Self::BUDDY_ORDER_MASK))
    }

    #[inline(always)]
    pub const fn get_buddy_order(&self) -> u8 {
        (self.0 >> 8) as u8
    }

    #[inline(always)]
    pub const fn with_buddy_order(&self, order: u8) -> Self {
        let cleared_order = self.0 & !Self::BUDDY_ORDER_MASK;
        Self(cleared_order | ((order as u16) << 8))
    }
}
