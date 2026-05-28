use alloc::vec::Vec;

use crate::{drivers::pci::{PCI_DEVICES, PCIDevice, config::pci_config_read_32}, util::bitwise::check_bit};

pub fn check_addr(bus: u8, slot: u8, func: u8, offset: u8) -> Option<PCIDevice> {
    let reg0 = pci_config_read_32(bus, slot, func, offset);
    let (device_id, vendor_id) = ((reg0 >> 16) as u16, (reg0 & 0xFFFF) as u16);
    if vendor_id == 0xFFFF {
        return None;
    } else {
        let reg2 = pci_config_read_32(bus, slot, func, 0x8);
        let (class, subclass) = ((reg2 >> 24) as u8, ((reg2 >> 16) & 0xFF) as u8);
        let reg3 = pci_config_read_32(bus, slot, func, 0xC);
        let header_type = ((reg3 >> 16) & 0xFF) as u8;
        let device = PCIDevice {
            bus,
            slot,
            func,
            vendor_id,
            device_id,
            class,
            subclass,
            header_type
        };
        Some(device)
    }
}

pub fn enumerate_pci_devices() {
    let mut global = PCI_DEVICES.lock();
    let mut devices = Vec::new();
    for bus in 0..=255 {
        for slot in 0..=31 {
            if let Some(dev) = check_addr(bus, slot, 0, 0) {
                let multifunc = check_bit(dev.header_type, 7);
                devices.push(dev);
                if multifunc {
                    for func in 1..=7 {
                        if let Some(dev) = check_addr(bus, slot, func, 0) {
                            devices.push(dev);
                        }
                    }
                }
            }
        }
    }
    *global = devices;
}
