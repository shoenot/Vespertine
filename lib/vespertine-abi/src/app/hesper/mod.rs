pub const HESPER_APP_METADATA_REQUEST: u32 = 0x4800;
pub const HESPER_APP_METADATA_RESPONSE: u32 = 0x4801;

pub const HESPER_EXECUTE_REQUEST: u32 = 0x4810;
pub const HESPER_EXECUTE_RESPONSE: u32 = 0x4811;

pub const HESPER_STATUS_OK: u32 = 0;
pub const HESPER_STATUS_NOT_FOUND: u32 = 1;
pub const HESPER_STATUS_INVALID_REQUEST: u32 = 2;
pub const HESPER_STATUS_LAUNCH_FAILED: u32 = 3;
pub const HESPER_STATUS_NOT_IMPLEMENTED: u32 = 4;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIoMode {
    Any = 0,
    Text = 1,
    Typed = 2,
    Direct = 3,
}
