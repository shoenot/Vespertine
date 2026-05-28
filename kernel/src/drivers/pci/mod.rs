use core::fmt::Display;

use crate::util::bitwise::check_bit;

mod config;
mod enumerate;
use alloc::vec::Vec;
pub use enumerate::enumerate_pci_devices;
pub use config::*;
use vespertine_common::lock::TicketLock;

pub static PCI_DEVICES: TicketLock<Vec<PCIDevice>> = TicketLock::new(Vec::new());

#[derive(Debug, Copy, Clone)]
pub struct PCIDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub header_type: u8,
}

impl Display for PCIDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[INFO] *PCI Device*: Bus: {}, Slot: {}, Function: {},\n                   Vendor ID: {:X}, Device ID: {:X},\n                   Class: {:X}, Subclass: {:X}",
                              self.bus, self.slot, self.func, self.vendor_id, self.device_id,
                              self.class, self.subclass)
    }
}

#[derive(Debug, Copy, Clone)]
pub enum PCIBar {
    Memory {
        quad: bool,     // can be mapped in  64 bit (quad) space
        prefetchable: bool,
        addr: u64,
        size: u64,
    },
    IOSpace {
        addr: u32,
        size: u64,
    }
}

fn probe_bar_size_mask(bus: u8, slot: u8, func: u8, offset: u8, orig: u32) -> u32 {
    let mask: u32;
    pci_config_write_32(bus, slot, func, offset, 0xFFFF_FFFF);
    mask = pci_config_read_32(bus, slot, func, offset);
    pci_config_write_32(bus, slot, func, offset, orig);
    mask
}

pub fn get_bar(dev: PCIDevice, bar_n: u8) -> PCIBar {
    let offset = 0x10 + (bar_n * 4);
    let rawbar = pci_config_read_32(dev.bus, dev.slot, dev.func, offset);

    // get bar type, address etc
    if !check_bit(rawbar, 0) {   // memory
        // 0x01 for 'type' is reserved so it can only be quad or not quad
        let quad = if ((rawbar >> 1) & 0b11) == 0x02 { true } else { false };
        let prefetchable = if ((rawbar >> 3) & 0b1) == 1 { true } else { false };
        let mut addr = (rawbar & !0xF) as u64;
        let mut mask = probe_bar_size_mask(dev.bus, dev.slot, dev.func, offset, rawbar) as u64;
        if quad {
            let offset = 0x10 + ((bar_n + 1) * 4);
            let upper = pci_config_read_32(dev.bus, dev.slot, dev.func, offset);
            addr |= (upper as u64) << 32;
            let himask = probe_bar_size_mask(dev.bus, dev.slot, dev.func, offset, upper) as u64;
            mask |= himask << 32;
        }
        mask &= !0xF;
        let size = (!mask) + 1;
        PCIBar::Memory { quad, prefetchable, addr, size }
    } else {                                    // io
        let addr = rawbar & !0x3;
        let mut mask = probe_bar_size_mask(dev.bus, dev.slot, dev.func, offset, rawbar);
        mask &= !0x3;
        let size = ((!mask) + 1) as u64;
        PCIBar::IOSpace { addr, size }
    }
}

pub fn pci_get_dev(vendor_id: u16, device_id: u16) -> Option<PCIDevice> {
    let devs = PCI_DEVICES.lock();
    for dev in &*devs {
        if dev.vendor_id == vendor_id && dev.device_id == device_id {
            return Some(dev.clone());
        }
    }
    None
}
