#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    String = 1,
    Integer = 2,
    Float = 3,
    Bool = 4,
    DateTime = 5,
    FileSize = 6,
    Record = 7,
    List = 8,
}

impl ValueType {
    pub const fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::String),
            2 => Some(Self::Integer),
            3 => Some(Self::Float),
            4 => Some(Self::Bool),
            5 => Some(Self::DateTime),
            6 => Some(Self::FileSize),
            7 => Some(Self::Record),
            8 => Some(Self::List),
            _ => None,
        }
    }
}

pub const RECORD_FIELD_NAME_MAX: usize = 128;

pub const RECORD_PRESENTATION_DEFAULT: u16 = 1;
pub const RECORD_PRESENTATION_TABLE: u16 = 2;

pub const DATETIME_HAS_OFFSET: u32 = 1 << 0;
pub const DATETIME_DATE_ONLY: u32 = 1 << 1;
pub const DATETIME_TIME_ONLY: u32 = 1 << 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSchemaHeader {
    pub schema_id: u64,
    pub field_count: u16,
    pub reserved: u16,
    pub payload_len: u32,
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
pub struct DateTimeValue {
    pub unix_seconds: i128,
    pub nanos: u32,
    pub offset_minutes: i32,
    pub flags: u32,
    pub calendar: u16,
    pub reserved: u16,
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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListHeader {
    pub element_type: u16,
    pub flags: u16,
    pub count: u32,
    pub payload_len: u32,
}
