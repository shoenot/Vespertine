use crate::arch::x86_64::io::{inb, outb};
use crate::kernel::time::ClockSource;

const PIT_BASE_FREQ: u32 = 1_193_182;

const PIT_CHANNEL_0: u16 = 0x40;
const PIT_CHANNEL_1: u16 = 0x41;
const PIT_CHANNEL_2: u16 = 0x42;
const PIT_COMMAND: u16 = 0x43;

#[derive(Copy, Clone, Debug)]
pub struct PIT {
    pub frequency: usize,
}

impl PIT {
    pub fn init_mode_2(&mut self, target_hz: u32) {
        self.frequency = target_hz as usize;
        let mut divisor = PIT_BASE_FREQ / target_hz;
        if divisor > 0xFFFF { divisor = 0 };
        unsafe {
            outb(PIT_COMMAND, 0b00110100);
            outb(PIT_CHANNEL_0, (divisor & 0xFF) as u8);
            outb(PIT_CHANNEL_0, (divisor >> 8) as u8);
        }
    }

    pub fn init_mode_0(&mut self) {
        self.frequency = 100;
        let mut divisor = 11932u32;
        if divisor > 0xFFFF { divisor = 0 };
        unsafe {
            outb(PIT_COMMAND, 0b00110000);
            outb(PIT_CHANNEL_0, (divisor & 0xFF) as u8);
            outb(PIT_CHANNEL_0, (divisor >> 8) as u8);
        }
    }
}

impl ClockSource for PIT {
    fn name(&self) -> &'static str {
        "PIT"
    }

    fn read_counter(&self) -> usize {
        let (lobyte, hibyte): (u8, u8);
        unsafe {
            outb(PIT_COMMAND, 0b0000_0000);
            lobyte = inb(PIT_CHANNEL_0);
            hibyte = inb(PIT_CHANNEL_0);
        }
        lobyte as usize | (hibyte as usize) << 8 
    }

    fn frequency(&self) -> usize {
        self.frequency
    }
}
