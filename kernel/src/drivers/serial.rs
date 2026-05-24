#![allow(dead_code)]

use core::hint::spin_loop;

use crate::arch::x86_64::io::{
    inb,
    outb,
};

const PORT: u16 = 0x3f8;

pub fn init_serial() {
    unsafe {
        outb(PORT + 1, 0x00); // disable all interrupts
        outb(PORT + 3, 0x80); // enable dlab (set baud rate divisor)
        outb(PORT + 0, 0x03); // set divisor to 3 (lo byte) 38400 baud 
        outb(PORT + 1, 0x00); //                  (hi byte)
        outb(PORT + 3, 0x03); // 8 bits, no parity, one stop bit
        outb(PORT + 2, 0xC7); // enable FIFO, clear them, with 14 byte threshold
        outb(PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set 
    }
}

fn wait_for_transmit() {
    unsafe {
        while (inb(PORT + 5) & 0x20) == 0 {
            spin_loop();
        }
    }
}

pub fn log_to_serial(s: &str) {
    unsafe {
        for b in s.bytes() {
            wait_for_transmit();
            outb(PORT + 0, b);
        }
    }
}

pub fn log_u32_to_serial(mut n: u32) {
    let mut buffer = [0u8; 16];
    let mut i = buffer.len();
    while n > 0 && i > 0 {
        i -= 1;
        buffer[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    let numstr = core::str::from_utf8(&buffer[i..]).unwrap();
    log_to_serial(&numstr);
}

pub fn log_u64_to_serial(mut n: u64) {
    let mut buffer = [0u8; 24];
    let mut i = buffer.len();
    while n > 0 && i > 0 {
        i -= 1;
        buffer[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    let numstr = core::str::from_utf8(&buffer[i..]).unwrap();
    log_to_serial(&numstr);
}

pub struct SerialWriter;
impl SerialWriter {
    pub fn new() -> SerialWriter { SerialWriter }
}

impl core::fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            if c == '\n' {
                log_to_serial("\n");
            } else {
                unsafe {
                    wait_for_transmit();
                    outb(PORT + 0, c as u8);
                }
            }
        }
        Ok(())
    }
}
