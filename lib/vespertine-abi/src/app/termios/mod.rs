mod bits;
pub use bits::*;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Termios {
    pub c_iflag:    u32,
    pub c_oflag:    u32,
    pub c_cflag:    u32,
    pub c_lflag:    u32,
    pub c_line:     u8,
    pub c_cc:       [u8; 32],
    pub c_ispeed:   u32,
    pub c_ospeed:   u32,
}

pub fn check_flag(field: u32, flag: u32) -> bool {
    (field & flag) != 0
}

impl Termios {
    pub const fn new() -> Self {
        let mut cc = [0u8; 32];
        cc[VINTR] = 0x03;
        cc[VEOF] = 0x04;
        Self {
            c_iflag: ICRNL | IXON,
            c_oflag: OPOST | ONLCR,
            c_cflag: CS8 | CREAD | CLOCAL,
            c_lflag: ISIG | ICANON | ECHO | ECHOE,
            c_line: 0,
            c_cc: cc,
            c_ispeed: B38400,
            c_ospeed: B38400,
        }
    }
}

impl Default for Termios {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum TermCommand {
    SetTermios(Termios),
    GetTermios,
    GetWindowSize,
}
