use core::arch::asm;

pub unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("outb %al, %dx",
             in("dx") port,
             in("al") value,
             options(att_syntax))
    }
}

pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("inb %dx, %al",
             in("dx") port,
             out("al") value,
             options(att_syntax))
    }
    value 
}
