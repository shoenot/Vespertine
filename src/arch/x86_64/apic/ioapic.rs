use core::ptr::{read_volatile, write_volatile};

const IOREGSEL_OFFSET: usize = 0x00;
const IOWIN_OFFSET: usize = 0x10;
const IOREDTBL_BASE: u8 = 0x10;

pub struct IoApic {
    base_addr: usize,
}

impl IoApic {
    pub unsafe fn new(base_addr: usize) -> Self {
       Self { base_addr }  
    }
}
