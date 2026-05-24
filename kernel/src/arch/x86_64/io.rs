#![allow(dead_code)]

use core::arch::asm;

// BYTE
#[inline(always)]
pub unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al",
             in("dx") port,
             in("al") value)
    }
}

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx",
             in("dx") port,
             out("al") value)
    }
    value
}

// LONG
#[inline(always)]
pub unsafe fn outl(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax",
             in("dx") port,
             in("eax") value)
    }
}

#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    unsafe {
        asm!("in eax, dx",
             in("dx") port,
             out("eax") value)
    }
    value
}

#[inline(always)]
pub unsafe fn read_cmos(index: u8) -> u8 {
    let idx_nmi = index | 0x80;
    unsafe {
        outb(0x70, idx_nmi);
        inb(0x71)
    }
}

#[inline(always)]
pub unsafe fn write_cmos(index: u8, value: u8) {
    let idx_nmi = (index & 0x3F) | 0x80;
    unsafe {
        outb(0x70, idx_nmi);
        outb(0x71, value);
    }
}
