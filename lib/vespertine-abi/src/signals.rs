use crate::{HandleID, define_bitflags};


define_bitflags! {
    pub struct Signal(u32) {
        READABLE    = 1 << 0;
        WRITABLE    = 1 << 1;
        PEER_CLOSED = 1 << 2;
        TERMINATED  = 1 << 3;
    }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct WaitItem {
    pub handle: HandleID,
    pub signal: Signal,
    pub pending: Signal,
}

