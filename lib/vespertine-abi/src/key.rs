use crate::define_bitflags;

define_bitflags! {
    pub struct Modifiers(u8) {
        SHIFT   = 1 << 0;
        ALT     = 1 << 1;
        CTRL    = 1 << 2;
        SUPER   = 1 << 3;
        CAPSL   = 1 << 4;
        NUML    = 1 << 5;
        SCRL    = 1 << 6;
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Released = 0,
    Pressed = 1,
    Repeat = 2,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Unknown = 0,
    
    // LETTERS 
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,

    // NUMBERS
    Num0, Num1, Num2, Num3, Num4,
    Num5, Num6, Num7, Num8, Num9,

    // FUNCTION
    F1, F2, F3, F4, F5, F6, 
    F7, F8, F9, F10, F11, F12,
    F13, F14, F15, F16, F17, F18, 
    F19, F20, F21, F22, F23, F24,

    // CONTROLS
    Enter, Backspace, Escape, Tab, Space,

    // MODIFIERS
    LeftShift, RightShift,
    LeftCtrl, RightCtrl,
    LeftAlt, RightAlt,          // Alt, AltGr
    LeftSuper, RightSuper,
    CapsLock, Menu,
    ScrollLock, NumLock,

    // NAVIGATION
    Up, Down, Left, Right,
    Home, End,
    PgUp, PgDown,
    Insert, Delete,

    // SYMBOLS
    Grave,          // ` ~
    Minus,          // - _
    Equal,          // = + 
    LeftBracket,    // [ {
    RightBracket,   // ] }
    Backslash,      // \ |
    Semicolon,       // ; : 
    Quote,          // ' "
    Comma,          // , <
    Period,         // . >
    Slash,          // / ?
    
    // NUMPAD
    Kp0, Kp1, Kp2, Kp3, Kp4,
    Kp5, Kp6, Kp7, Kp8, Kp9,
    KpDecimal, KpEqual, KpEnter,
    KpDivide, KpMultiply, KpSubtract, KpAdd,

    // SYSTEM
    PrintScreen, Pause, Power, Sleep, Wake,

    // MEDIA
    MediaPlayPause,
    MediaStop,
    MediaNextTrack,
    MediaPrevTrack,
    VolumeMute,
    VolumeUp,
    VolumeDown,
    MediaSelect,

    // APPLICATION
    LaunchMail,
    LaunchCalculator,
    BrowserHome,
    BrowserBack,
    BrowserForward,
    BrowserRefresh,
    BrowserStop,
    BrowserSearch,
    BrowserFavorites,

    // INTERNATIONAL
    IntlBackslash,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub state: KeyState,
    pub mods: Modifiers,
    pub unicode: char,
}

impl KeyEvent {
    pub const fn zeroed() -> Self {
        Self { code: KeyCode::Unknown, state: KeyState::Released, mods: Modifiers::new(), unicode: '\0' }
    }

    #[inline(always)]
    pub fn is_press(&self) -> bool {
        matches!(self.state, KeyState::Pressed | KeyState::Repeat)
    }

    #[inline(always)]
    pub fn is_release(&self) -> bool {
        self.state == KeyState::Released
    }
}
