use crate::define_bitflags;

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
    Terminal = 3,
}

define_bitflags! {
    pub struct AppIoModes(u8) {
        TEXT          = 1 << 0;
        TYPED         = 1 << 1;
        TERMINAL      = 1 << 2;
    }
}

impl AppIoModes {
    pub fn from_mode(mode: AppIoMode) -> Self {
        match mode {
            AppIoMode::Any => AppIoModes::TEXT | AppIoModes::TYPED | AppIoModes::TERMINAL,
            AppIoMode::Text => AppIoModes::TEXT,
            AppIoMode::Typed => AppIoModes::TYPED,
            AppIoMode::Terminal => AppIoModes::TERMINAL,
        }
    }

    pub fn contains_mode(self, mode: AppIoMode) -> bool {
        match mode {
            AppIoMode::Any => true,
            AppIoMode::Text => self.contains(AppIoModes::TEXT),
            AppIoMode::Typed => self.contains(AppIoModes::TYPED),
            AppIoMode::Terminal => self.contains(AppIoModes::TERMINAL),
        }
    }
}
