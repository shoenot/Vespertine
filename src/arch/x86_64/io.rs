use core::arch::asm;

// BYTE
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

// LONG
pub unsafe fn outl(port: u16, value: u32) {
    unsafe {
        asm!("outl %eax, %dx",
             in("dx") port,
             in("eax") value,
             options(att_syntax))
    }
}

pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    unsafe {
        asm!("inl %dx, %eax",
             in("dx") port,
             out("eax") value,
             options(att_syntax))
    }
    value 
}
