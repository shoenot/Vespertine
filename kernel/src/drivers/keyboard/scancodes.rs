use vespertine_abi::key::{KeyCode, Modifiers};

use crate::drivers::keyboard::keymaps::Keymap_US;

pub static mut ACTIVE_KEYMAP: &'static dyn Keymap = &Keymap_US;

pub const KBD_US_BASE: [char; 128] = [
    '\0', '\x1B', '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-', '=', '\x08', '\t', // 0x00 - 0x0F
    'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', '[', ']', '\r', '\0', 'a', 's', // 0x10 - 0x1F
    'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', '\'', '`', '\0', '\\', 'z', 'x', 'c', 'v', // 0x20 - 0x2F
    'b', 'n', 'm', ',', '.', '/', '\0', '*', '\0', ' ', '\0', '\0', '\0', '\0', '\0', '\0', // 0x30 - 0x3F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '7', '8', '9', '-', '4', '5', '6', '+', '1', // 0x40 - 0x4F
    '2', '3', '0', '.', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x50 - 0x5F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x60 - 0x6F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x70 - 0x7F
];

pub const KBD_US_SHIFT: [char; 128] = [
    '\0', '\x1B', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_', '+', '\x08', '\t', // 0x00 - 0x0F
    'Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P', '{', '}', '\r', '\0', 'A', 'S', // 0x10 - 0x1F
    'D', 'F', 'G', 'H', 'J', 'K', 'L', ':', '"', '~', '\0', '|', 'Z', 'X', 'C', 'V', // 0x20 - 0x2F
    'B', 'N', 'M', '<', '>', '?', '\0', '*', '\0', ' ', '\0', '\0', '\0', '\0', '\0', '\0', // 0x30 - 0x3F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '-', '\0', '\0', '\0', '+', '\0', // 0x40 - 0x4F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x50 - 0x5F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x60 - 0x6F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x70 - 0x7F
];

pub const KBD_US_EXTENDED: [char; 128] = [
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x00 - 0x0F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\r', '\0', '\0',
    '\0', // 0x10 - 0x1F (0x1C = Numpad Enter)
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x20 - 0x2F
    '\0', '\0', '\0', '\0', '\0', '/', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x30 - 0x3F (0x35 = Numpad /)
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\x11', '\0', '\0', '\x13', '\0', '\x14', '\0', '\0', // 0x40 - 0x4F
    '\x12', '\0', '\0', '\x7F', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x50 - 0x5F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x60 - 0x6F
    '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', '\0', // 0x70 - 0x7F
];

pub trait Keymap: Sync + Send {
    fn name(&self) -> &'static str;
    fn map_key(&self, code: KeyCode, mods: Modifiers) -> char;
}

pub fn set_keymap(map: &'static dyn Keymap) {
    unsafe {
        ACTIVE_KEYMAP = map;
    }
}

pub fn active_keymap() -> &'static dyn Keymap {
    unsafe { ACTIVE_KEYMAP }
}

pub fn scancode_to_keycode(scancode: usize, is_extended: bool) -> KeyCode {
    if is_extended {
        match scancode {
            0x1C => KeyCode::KpEnter,
            0x1D => KeyCode::RightCtrl,
            0x35 => KeyCode::KpDivide,
            0x37 => KeyCode::PrintScreen,
            0x38 => KeyCode::RightAlt,
            0x47 => KeyCode::Home,
            0x48 => KeyCode::Up,
            0x49 => KeyCode::PgUp,
            0x4B => KeyCode::Left,
            0x4D => KeyCode::Right,
            0x4F => KeyCode::End,
            0x50 => KeyCode::Down,
            0x51 => KeyCode::PgDown,
            0x52 => KeyCode::Insert,
            0x53 => KeyCode::Delete,
            0x5B => KeyCode::LeftSuper,
            0x5C => KeyCode::RightSuper,
            0x5D => KeyCode::Menu,
            0x5E => KeyCode::Power,
            0x5F => KeyCode::Sleep,
            0x63 => KeyCode::Wake,
            // MULTIMEDIA
            0x20 => KeyCode::VolumeMute,
            0x2E => KeyCode::VolumeDown,
            0x30 => KeyCode::VolumeUp,
            0x22 => KeyCode::MediaPlayPause,
            0x24 => KeyCode::MediaStop,
            0x10 => KeyCode::MediaPrevTrack,
            0x19 => KeyCode::MediaNextTrack,
            0x6D => KeyCode::MediaSelect,
            // APP / BROWSER
            0x6A => KeyCode::BrowserBack,
            0x69 => KeyCode::BrowserForward,
            0x67 => KeyCode::BrowserRefresh,
            0x68 => KeyCode::BrowserStop,
            0x65 => KeyCode::BrowserSearch,
            0x66 => KeyCode::BrowserFavorites,
            0x32 => KeyCode::BrowserHome,
            0x6C => KeyCode::LaunchMail,
            0x21 => KeyCode::LaunchCalculator,
            _ => KeyCode::Unknown,
        }
    } else {
        match scancode {
            0x01 => KeyCode::Escape,
            0x02 => KeyCode::Num1,
            0x03 => KeyCode::Num2,
            0x04 => KeyCode::Num3,
            0x05 => KeyCode::Num4,
            0x06 => KeyCode::Num5,
            0x07 => KeyCode::Num6,
            0x08 => KeyCode::Num7,
            0x09 => KeyCode::Num8,
            0x0A => KeyCode::Num9,
            0x0B => KeyCode::Num0,
            0x0C => KeyCode::Minus,
            0x0D => KeyCode::Equal,
            0x0E => KeyCode::Backspace,
            0x0F => KeyCode::Tab,
            0x10 => KeyCode::Q,
            0x11 => KeyCode::W,
            0x12 => KeyCode::E,
            0x13 => KeyCode::R,
            0x14 => KeyCode::T,
            0x15 => KeyCode::Y,
            0x16 => KeyCode::U,
            0x17 => KeyCode::I,
            0x18 => KeyCode::O,
            0x19 => KeyCode::P,
            0x1A => KeyCode::LeftBracket,
            0x1B => KeyCode::RightBracket,
            0x1C => KeyCode::Enter,
            0x1D => KeyCode::LeftCtrl,
            0x1E => KeyCode::A,
            0x1F => KeyCode::S,
            0x20 => KeyCode::D,
            0x21 => KeyCode::F,
            0x22 => KeyCode::G,
            0x23 => KeyCode::H,
            0x24 => KeyCode::J,
            0x25 => KeyCode::K,
            0x26 => KeyCode::L,
            0x27 => KeyCode::Semicolon,
            0x28 => KeyCode::Quote,
            0x29 => KeyCode::Grave,
            0x2A => KeyCode::LeftShift,
            0x2B => KeyCode::Backslash,
            0x2C => KeyCode::Z,
            0x2D => KeyCode::X,
            0x2E => KeyCode::C,
            0x2F => KeyCode::V,
            0x30 => KeyCode::B,
            0x31 => KeyCode::N,
            0x32 => KeyCode::M,
            0x33 => KeyCode::Comma,
            0x34 => KeyCode::Period,
            0x35 => KeyCode::Slash,
            0x36 => KeyCode::RightShift,
            0x37 => KeyCode::KpMultiply,
            0x38 => KeyCode::LeftAlt,
            0x39 => KeyCode::Space,
            0x3A => KeyCode::CapsLock,
            0x3B => KeyCode::F1,
            0x3C => KeyCode::F2,
            0x3D => KeyCode::F3,
            0x3E => KeyCode::F4,
            0x3F => KeyCode::F5,
            0x40 => KeyCode::F6,
            0x41 => KeyCode::F7,
            0x42 => KeyCode::F8,
            0x43 => KeyCode::F9,
            0x44 => KeyCode::F10,
            0x45 => KeyCode::NumLock,
            0x46 => KeyCode::ScrollLock,
            0x47 => KeyCode::Kp7,
            0x48 => KeyCode::Kp8,
            0x49 => KeyCode::Kp9,
            0x4A => KeyCode::KpSubtract,
            0x4B => KeyCode::Kp4,
            0x4C => KeyCode::Kp5,
            0x4D => KeyCode::Kp6,
            0x4E => KeyCode::KpAdd,
            0x4F => KeyCode::Kp1,
            0x50 => KeyCode::Kp2,
            0x51 => KeyCode::Kp3,
            0x52 => KeyCode::Kp0,
            0x53 => KeyCode::KpDecimal,
            0x56 => KeyCode::IntlBackslash,
            0x57 => KeyCode::F11,
            0x58 => KeyCode::F12,
            _ => KeyCode::Unknown,
        }
    }
}
