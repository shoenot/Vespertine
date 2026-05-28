use crate::arch::x86_64::io::{inl, outl};

pub fn pci_build_addr(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    (bus as u32) << 16 |
    (slot as u32) << 11 |
    (func as u32) << 8 |
    (offset & 0xFC) as u32 |
    0x8000_0000
}

pub fn pci_config_read_16(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    let addr = pci_build_addr(bus, slot, func, offset);
    unsafe { outl(0xCF8, addr) };
    let res = unsafe { inl(0xCFC) };
    ((res >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

pub fn pci_config_read_32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let addr = pci_build_addr(bus, slot, func, offset);
    unsafe { outl(0xCF8, addr) };
    unsafe { inl(0xCFC) }
}

pub fn pci_config_write_32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let addr = pci_build_addr(bus, slot, func, offset);
    unsafe { outl(0xCF8, addr) };
    unsafe { outl(0xCFC, value) };
}
