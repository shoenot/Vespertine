
pub const DATETIME_HAS_OFFSET: u32 = 1 << 0;
pub const DATETIME_DATE_ONLY: u32 = 1 << 1;
pub const DATETIME_TIME_ONLY: u32 = 1 << 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateValue {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub calendar: u16,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeValue {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub reserved: u8,
    pub nanos: u32,
    pub offset_minutes: i32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeValue {
    pub unix_seconds: i64,
    pub nanos: u32,
    pub offset_minutes: i32,
    pub flags: u32,
    pub calendar: u16,
    pub reserved: u16,
}

