pub const HESPER_APP_METADATA_REQUEST: u32 = 0x4800;
pub const HESPER_APP_METADATA_RESPONSE: u32 = 0x4801;

pub const HESPER_STATUS_OK: u32 = 0;
pub const HESPER_STATUS_NOT_FOUND: u32 = 1;
pub const HESPER_STATUS_INVALID_REQUEST: u32 = 2;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIoMode {
    Any = 0,
    Text = 1,
    Typed = 2,
    Direct = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AppMetadataRequestHeader {
    pub app_name_len: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AppMetadataResponseHeader {
    pub status: u32,
    pub input: u8,
    pub output: u8,
    pub flags: u16,
    pub app_id_len: u32,
    pub display_name_len: u32,
}
