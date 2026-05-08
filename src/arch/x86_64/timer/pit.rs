use crate::arch::x86_64::io::{self, outb};

const PIT_BASE_FREQ: u32 = 1_193_182;

const PIT_CHANNEL_0: u16 = 0x40;
const PIT_CHANNEL_1: u16 = 0x41;
const PIT_CHANNEL_2: u16 = 0x42;
const PIT_COMMAND: u16 = 0x43;

pub fn init(target_hz: u32) {
    let mut divisor = PIT_BASE_FREQ / target_hz;
    if divisor > 0xFFFF { divisor = 0 };
    unsafe {
        outb(PIT_COMMAND, 0b00110100);
        outb(PIT_CHANNEL_0, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL_0, (divisor >> 8) as u8);
    }
}
