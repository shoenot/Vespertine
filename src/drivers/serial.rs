use crate::{asm, hcf};

unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("outb %al, %dx",
             in("dx") port,
             in("al") value,
             options(att_syntax))
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("inb %dx, %al",
             in("dx") port,
             out("al") value,
             options(att_syntax))
    }
    value 
}

const PORT: u16 = 0x3f8;

pub unsafe fn init_serial() {
    unsafe {
        outb(PORT + 1, 0x00);  // disable all interrupts
        outb(PORT + 3, 0x80);  // enable dlab (set baud rate divisor)
        outb(PORT + 0, 0x03);  // set divisor to 3 (lo byte) 38400 baud 
        outb(PORT + 1, 0x00);  //                  (hi byte)
        outb(PORT + 3, 0x03);  // 8 bits, no parity, one stop bit
        outb(PORT + 2, 0xC7);  // enable FIFO, clear them, with 14 byte threshold
        outb(PORT + 4, 0x0B);  // IRQs enabled, RTS/DSR set 
        outb(PORT + 4, 0x1E);  // Set in loopback mode, test the serial chip
        outb(PORT + 0, 0xAE);  // send a test byte 
        
        if inb(PORT + 0) != 0xAE {
            hcf();
        }

        outb(PORT + 4, 0x0F);
    }
}

pub fn log_to_serial(s: &str) {
    unsafe {
        s.bytes().for_each(|b| outb(PORT + 0, b));
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
