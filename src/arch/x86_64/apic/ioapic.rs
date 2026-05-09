use core::ptr::{read_volatile, write_volatile};
use crate::kernel::{acpi, memory::pmm::HHDMOFFSET};

const IOREGSEL_OFFSET: usize = 0x00;
const IOWIN_OFFSET: usize = 0x10;
const IOREDTBL_BASE: u8 = 0x10;

pub struct IOApic {
    pub base_addr: usize,
    pub gsi_base: usize,
}

pub fn get_ioapic_addrs() -> (usize, usize) {
    let rsdp = acpi::rsdp::Rsdp::get();
    let sdt = acpi::sdt::SDTArray::get(rsdp.get_table());
    let madt = acpi::madt::parse_madt(&sdt);
    let io_apic = &madt.io_apics[0];
    (io_apic.addr as usize, io_apic.gsi_base as usize)
}

impl IOApic {
    pub fn init(&mut self, addr: usize, gsi_base: usize) {
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
    pub fn set_entry(&self, gsi: u32, vector: u8, lapic_id: u32) {
        if gsi < self.gsi_base as u32 { return; }

        let rel_gsi = (gsi - self.gsi_base as u32) as u8;
        let low_idx = IOREDTBL_BASE + (rel_gsi * 2);
        let high_idx = IOREDTBL_BASE + (rel_gsi * 2) + 1;

        // Low 32 bits: 
        // - Bits 0-7: Interrupt Vector
        // - Bits 8-10: Delivery Mode (000 = Fixed)
        // - Bit 11: Destination Mode (0 = Physical)
        // - Bit 13: Pin Polarity (0 = High active)
        // - Bit 15: Trigger Mode (0 = Edge)
        // - Bit 16: Mask (0 = Unmasked/Enabled)
        let low_val = vector as u32;

        // High 32 bits:
        // - Bits 24-31 (Bits 56-63 of the 64-bit entry): Destination LAPIC ID
        let high_val = lapic_id << 24;

        unsafe {
            self.write_reg(low_idx, low_val);
            self.write_reg(high_idx, high_val);
        }
    }
}
