mod dt;
pub use dt::*;

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    String = 1,
    Integer = 2,
    Float = 3,
    Bool = 4,
    Date = 5,
    Time = 6,
    DateTime = 7,
    FileSize = 8,
    Record = 9,
    List = 10,
    User = 11,
}

impl ValueType {
    pub const fn as_u16(self) -> u16 { self as u16 }

    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::String),
            2 => Some(Self::Integer),
            3 => Some(Self::Float),
            4 => Some(Self::Bool),
            5 => Some(Self::Date),
            6 => Some(Self::Time),
            7 => Some(Self::DateTime),
            8 => Some(Self::FileSize),
            9 => Some(Self::Record),
            10 => Some(Self::List),
            11 => Some(Self::User),
            _ => None,
        }
    }
}

pub const RECORD_FIELD_NAME_MAX: usize = 4096;

pub const RECORD_PRESENTATION_DEFAULT: u16 = 0;
pub const RECORD_PRESENTATION_TABLE: u16 = 1;
pub const RECORD_PRESENTATION_DETAILS: u16 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSchemaHeader {
    pub schema_id: u64,
    pub field_count: u16,
    pub reserved: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordField {
    pub value_type: u16,
    pub name_len: u8,
    pub reserved: u8,
    pub name: [u8; RECORD_FIELD_NAME_MAX],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordPresentationHeader {
    pub schema_id: u64,
    pub presentation: u16,
    pub field_count: u16,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordValueHeader {
    pub schema_id: u64,
    pub field_count: u16,
    pub flags: u16,
    pub payload_len: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueHeader {
    pub value_type: u16,
    pub flags: u16,
    pub payload_len: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSizeValue {
    pub bytes: i128,
    pub block_size: u64,
    pub blocks: i128,
    pub flags: u32,
    pub reserved: u32,
}

pub const USER_NAME_MAX: usize = 64;
pub const USER_DISPLAY_NAME_MAX: usize = 255;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserValue {
    pub id: u32,
    pub name_len: u8,
    pub display_name_len: u8,
    pub first_name_len: u8,
    pub last_name_len: u8,
    pub flags: u16,
    pub reserved: u32,
    pub name: [u8; USER_NAME_MAX],
    pub display_name: [u8; USER_DISPLAY_NAME_MAX],
    pub first_name: [u8; USER_DISPLAY_NAME_MAX],
    pub last_name: [u8; USER_DISPLAY_NAME_MAX],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListHeader {
    pub element_type: u16,
    pub flags: u16,
    pub count: u32,
    pub payload_len: u32,
}

pub const STREAM_INTENT_DEFAULT: u16 = 0;
pub const STREAM_INTENT_LIST: u16 = 1;
pub const STREAM_INTENT_DETAILS: u16 = 2;
pub const STREAM_INTENT_TABLE: u16 = 3;
pub const STREAM_INTENT_LOG: u16 = 4;
pub const STREAM_INTENT_METRICS: u16 = 5;
pub const STREAM_INTENT_CHOICES: u16 = 6;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamIntentHeader {
    pub intent: u16,
    pub flags: u16,
    pub reserved: u32,
}
