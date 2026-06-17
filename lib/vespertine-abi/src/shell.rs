#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    String = 1,
    Record = 2,
}

pub const RECORD_FIELD_NAME_MAX: usize = 128;

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
    pub value_type: ValueType,
    pub name_len: u8,
    pub reserved: u8,
    pub name: [u8; RECORD_FIELD_NAME_MAX],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    pub schema_id: u64,
    pub field_count: u16,
    pub reserved: u16,
}
