use core::ptr::{
    read_volatile,
    write_volatile,
};

use crate::core::acpi;
use crate::memory::HHDMOFFSET;

const IOREGSEL_OFFSET: usize = 0x00;
const IOWIN_OFFSET: usize = 0x10;
const IOREDTBL_BASE: u8 = 0x10;

pub struct IOApic {
    pub(crate) base_addr: usize,
    pub(crate) gsi_base: usize,
}

pub fn get_ioapic_addrs() -> (usize, usize) {
    let rsdp = acpi::rsdp::Rsdp::get();
    let sdt = acpi::sdt::SDTArray::get(rsdp.get_table());
    let madt = acpi::madt::parse_madt(&sdt);
    let io_apic = &madt.io_apics[0];
    (io_apic.addr as usize, io_apic.gsi_base as usize)
}

impl IOApic {
    pub(crate) fn init(&mut self, addr: usize, gsi_base: usize) {
        self.base_addr = addr as usize + *HHDMOFFSET;
        self.gsi_base = gsi_base as usize;
    }

    unsafe fn write_reg(&self, reg: u8, value: u32) {
        let sel_ptr = (self.base_addr + IOREGSEL_OFFSET) as *mut u32;
        let win_ptr = (self.base_addr + IOWIN_OFFSET) as *mut u32;
        unsafe {
            write_volatile(sel_ptr, reg as u32);
            write_volatile(win_ptr, value);
        }
    }

    unsafe fn read_reg(&self, reg: u8) -> u32 {
        let sel_ptr = (self.base_addr + IOREGSEL_OFFSET) as *mut u32;
        let win_ptr = (self.base_addr + IOWIN_OFFSET) as *mut u32;
        unsafe {
            write_volatile(sel_ptr, reg as u32);
            read_volatile(win_ptr)
        }
    }

    // route hw interrupt to a local apic mapped to a specific interrupt vector
    pub(crate) fn mask_all(&self) {
        // Read the Version Register at offset 0x01
        let version_reg = unsafe { self.read_reg(0x01) };

        // Extract the "Max Redirection Entry" field (bits 16-23)
        let max_entry = ((version_reg >> 16) & 0xFF) as u8;

        // Loop from 0 up to and including max_entry
        for i in 0..=max_entry {
            let low_idx = IOREDTBL_BASE + (i * 2);
            let high_idx = IOREDTBL_BASE + (i * 2) + 1;

            unsafe {
                // Write with the Mask Bit (16) set to 1
                self.write_reg(low_idx, 1 << 16);
                self.write_reg(high_idx, 0);
            }
        }
    }

    pub(crate) fn set_entry(&self, gsi: u32, vector: u8, lapic_id: u32, masked: bool, active_high: bool, edge_triggered: bool) {
        if gsi < self.gsi_base as u32 {
            return;
        }

        let rel_gsi = (gsi - self.gsi_base as u32) as u8;
        let low_idx = IOREDTBL_BASE + (rel_gsi * 2);
        let high_idx = IOREDTBL_BASE + (rel_gsi * 2) + 1;

        // Bits 0-7: Vector
        // Bit 13: Interrupt pin polarity (0: Active High, 1: Active Low)
        // Bit 15: Trigger mode (0: Edge triggered, 1: Level triggered)
        // Bit 16: Mask (1 = Disabled, 0 = Enabled)
        let mut low_val = vector as u32;
        if masked {
            low_val |= 1 << 16;
        }
        if !active_high {
            low_val |= 1 << 13;
        }
        if !edge_triggered {
            low_val |= 1 << 15;
        }

        // Bits 56-63 (Shifted): Destination LAPIC ID
        let high_val = lapic_id << 24;

        unsafe {
            self.write_reg(low_idx, low_val);
            self.write_reg(high_idx, high_val);
        }
    }
}
